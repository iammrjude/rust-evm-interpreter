use crate::memory::Memory;
use crate::opcodes::{
    dup_index, is_dup, is_push, is_swap, log_topics, opcode_name, push_size, static_gas_cost,
    swap_index,
};
use crate::stack::{Stack, StackError};
use crate::storage::Storage;
use primitive_types::{U256, U512};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use tiny_keccak::{Hasher, Keccak};

const MAX_CALL_DEPTH: usize = 1024;
const G_MEMORY: u64 = 3;
const G_COPY: u64 = 3;
const G_SHA3WORD: u64 = 6;
const G_LOGDATA: u64 = 8;
const G_EXPBYTE: u64 = 50;
const WARM_STORAGE_READ_COST: u64 = 100;
const COLD_SLOAD_COST: u64 = 2100;
const COLD_ACCOUNT_ACCESS_COST: u64 = 2600;
const G_CALLVALUE: u64 = 9000;
const G_NEWACCOUNT: u64 = 25000;
const G_CALLSTIPEND: u64 = 2300;
const G_CREATE_CODE_DEPOSIT: u64 = 200;
const G_INITCODE_WORD: u64 = 2;
const MAX_INITCODE_SIZE: usize = 49152;
const SSTORE_SET_GAS: u64 = 20000;
const SSTORE_RESET_GAS: u64 = 2900;
const SLOAD_GAS: u64 = 100;
const SSTORE_STIPEND: u64 = 2300;
const SSTORE_CLEARS_SCHEDULE: i64 = 4800;
const MAX_REFUND_QUOTIENT: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    Stop,
    Return,
    Revert,
    SelfDestruct,
    OutOfGas,
    StackOverflow,
    StackUnderflow,
    BadJumpDestination(usize),
    InvalidOpcode(u8),
    ReturnDataOutOfBounds,
    StaticModeViolation,
}

#[derive(Debug, Clone, Default)]
pub struct LogEntry {
    pub address: U256,
    pub topics: Vec<U256>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub address: U256,
    pub origin: U256,
    pub caller: U256,
    pub callvalue: U256,
    pub gas_price: U256,
    pub coinbase: U256,
    pub timestamp: U256,
    pub block_number: U256,
    pub prevrandao: U256,
    pub block_gas_limit: U256,
    pub chain_id: U256,
    pub self_balance: U256,
    pub base_fee: U256,
    pub block_hashes: HashMap<U256, U256>,
    pub account_balances: HashMap<U256, U256>,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            address: U256::zero(),
            origin: U256::zero(),
            caller: U256::zero(),
            callvalue: U256::zero(),
            gas_price: U256::zero(),
            coinbase: U256::zero(),
            timestamp: U256::zero(),
            block_number: U256::zero(),
            prevrandao: U256::zero(),
            block_gas_limit: U256::from(30_000_000u64),
            chain_id: U256::one(),
            self_balance: U256::zero(),
            base_fee: U256::zero(),
            block_hashes: HashMap::new(),
            account_balances: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub status: ExitStatus,
    pub stack: Vec<U256>,
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    pub gas_remaining: u64,
    pub logs: Vec<LogEntry>,
}

#[derive(Debug, Clone, Default)]
struct AccountState {
    balance: U256,
    nonce: u64,
    code: Vec<u8>,
    storage: Storage,
    created_in_tx: bool,
}

#[derive(Debug, Clone, Default)]
struct WorldState {
    accounts: HashMap<U256, AccountState>,
}

impl WorldState {
    fn account_mut(&mut self, address: U256) -> &mut AccountState {
        self.accounts.entry(address).or_default()
    }

    fn account(&self, address: U256) -> Option<&AccountState> {
        self.accounts.get(&address)
    }

    fn balance_of(&self, address: U256) -> U256 {
        self.account(address)
            .map(|account| account.balance)
            .unwrap_or_else(U256::zero)
    }

    fn set_balance(&mut self, address: U256, balance: U256) {
        self.account_mut(address).balance = balance;
    }

    fn exists(&self, address: U256) -> bool {
        if let Some(account) = self.account(address) {
            return account.nonce != 0
                || !account.balance.is_zero()
                || !account.code.is_empty()
                || !account.storage.is_empty();
        }
        false
    }

    fn transfer(&mut self, from: U256, to: U256, value: U256) -> bool {
        if value.is_zero() || from == to {
            return true;
        }

        let from_balance = self.balance_of(from);
        if from_balance < value {
            return false;
        }

        self.account_mut(from).balance = from_balance - value;
        let to_balance = self.balance_of(to);
        self.account_mut(to).balance = to_balance.overflowing_add(value).0;
        true
    }

    fn code_of(&self, address: U256) -> Vec<u8> {
        self.account(address)
            .map(|account| account.code.clone())
            .unwrap_or_default()
    }

    fn set_code(&mut self, address: U256, code: Vec<u8>) {
        let account = self.account_mut(address);
        account.code = code;
        account.created_in_tx = false;
    }

    fn deploy_code(&mut self, address: U256, code: Vec<u8>) {
        let account = self.account_mut(address);
        account.code = code;
        account.created_in_tx = true;
    }

    fn storage_load(&self, address: U256, key: U256) -> U256 {
        self.account(address)
            .map(|account| account.storage.load(key))
            .unwrap_or_else(U256::zero)
    }

    fn storage_store(&mut self, address: U256, key: U256, value: U256) {
        self.account_mut(address).storage.store(key, value);
    }

    fn increment_nonce(&mut self, address: U256) -> u64 {
        let account = self.account_mut(address);
        let current = account.nonce;
        account.nonce = account.nonce.wrapping_add(1);
        current
    }

    fn selfdestruct(&mut self, address: U256, beneficiary: U256) {
        let created_in_tx = self
            .account(address)
            .map(|account| account.created_in_tx)
            .unwrap_or(false);

        let balance = self.balance_of(address);
        if created_in_tx {
            if beneficiary != address {
                let _ = self.transfer(address, beneficiary, balance);
            } else {
                self.account_mut(address).balance = U256::zero();
            }
            let account = self.account_mut(address);
            account.code.clear();
            account.storage = Storage::new();
        } else if beneficiary != address && !balance.is_zero() {
            let _ = self.transfer(address, beneficiary, balance);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FrameContext {
    address: U256,
    caller: U256,
    callvalue: U256,
    is_static: bool,
    depth: usize,
}

#[derive(Debug, Clone, Copy)]
enum CallKind {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

#[derive(Debug, Clone)]
pub struct Evm {
    code: Vec<u8>,
    pc: usize,
    stack: Stack,
    memory: Memory,
    calldata: Vec<u8>,
    gas_limit: u64,
    gas_remaining: u64,
    return_data: Vec<u8>,
    last_return_data: Vec<u8>,
    jumpdests: HashSet<usize>,
    pub env: Env,
    world: WorldState,
    original_storage: HashMap<(U256, U256), U256>,
    accessed_addresses: HashSet<U256>,
    accessed_storage_keys: HashSet<(U256, U256)>,
    gas_refund: i64,
    logs: Vec<LogEntry>,
    trace: bool,
    context: FrameContext,
}

impl Evm {
    pub fn new(code: Vec<u8>) -> Self {
        let env = Env::default();
        let mut world = WorldState::default();
        world.set_code(env.address, code.clone());
        world.set_balance(env.address, env.self_balance);
        for (address, balance) in &env.account_balances {
            world.set_balance(*address, *balance);
        }

        let jumpdests = collect_jumpdests(&code);
        let gas_limit = 10_000_000u64;

        let mut vm = Self {
            code,
            pc: 0,
            stack: Stack::new(),
            memory: Memory::new(),
            calldata: Vec::new(),
            gas_limit,
            gas_remaining: gas_limit,
            return_data: Vec::new(),
            last_return_data: Vec::new(),
            jumpdests,
            world,
            original_storage: HashMap::new(),
            accessed_addresses: HashSet::new(),
            accessed_storage_keys: HashSet::new(),
            gas_refund: 0,
            env,
            logs: Vec::new(),
            trace: false,
            context: FrameContext {
                address: U256::zero(),
                caller: U256::zero(),
                callvalue: U256::zero(),
                is_static: false,
                depth: 0,
            },
        };
        vm.reset_access_lists();
        vm
    }

    pub fn with_calldata(mut self, calldata: Vec<u8>) -> Self {
        self.calldata = calldata;
        self
    }

    pub fn with_trace(mut self, trace: bool) -> Self {
        self.trace = trace;
        self
    }

    pub fn with_gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self.gas_remaining = gas_limit;
        self
    }

    pub fn with_env(mut self, env: Env) -> Self {
        self.context.address = env.address;
        self.context.caller = env.caller;
        self.context.callvalue = env.callvalue;
        self.context.is_static = false;
        self.context.depth = 0;

        self.world.set_code(env.address, self.code.clone());
        self.world.set_balance(env.address, env.self_balance);
        for (address, balance) in &env.account_balances {
            self.world.set_balance(*address, *balance);
        }

        self.env = env;
        self.reset_access_lists();
        self
    }

    pub fn with_external_code(mut self, address: U256, code: Vec<u8>) -> Self {
        self.world.set_code(address, code);
        self
    }

    pub fn with_account_balance(mut self, address: U256, balance: U256) -> Self {
        self.world.set_balance(address, balance);
        self
    }

    pub fn run(&mut self) -> ExecutionResult {
        let mut status = ExitStatus::Stop;

        while self.pc < self.code.len() {
            let pc_before = self.pc;
            let opcode = self.code[self.pc];
            self.pc += 1;

            let cost = static_gas_cost(opcode);
            if self.gas_remaining < cost {
                status = ExitStatus::OutOfGas;
                self.gas_remaining = 0;
                break;
            }
            self.gas_remaining -= cost;

            if self.trace {
                self.print_trace(pc_before, opcode);
            }

            match self.execute_opcode(opcode, pc_before) {
                Ok(Some(halt_status)) => {
                    status = halt_status;
                    break;
                }
                Ok(None) => {}
                Err(fault) => {
                    status = fault;
                    self.gas_remaining = 0;
                    break;
                }
            }
        }

        let mut gas_remaining = self.gas_remaining;
        if self.context.depth == 0 && is_success_status(&status) {
            let gas_used = self.gas_limit.saturating_sub(gas_remaining);
            let refund_cap = gas_used / MAX_REFUND_QUOTIENT;
            let refund = self.gas_refund.max(0) as u64;
            let applied_refund = refund.min(refund_cap);
            gas_remaining = gas_remaining.saturating_add(applied_refund);
        }

        ExecutionResult {
            status,
            stack: self.stack.as_slice().to_vec(),
            return_data: self.return_data.clone(),
            gas_used: self.gas_limit.saturating_sub(gas_remaining),
            gas_remaining,
            logs: self.logs.clone(),
        }
    }

    fn execute_opcode(
        &mut self,
        opcode: u8,
        pc_before: usize,
    ) -> Result<Option<ExitStatus>, ExitStatus> {
        match opcode {
            0x00 => Ok(Some(ExitStatus::Stop)),
            op if is_push(op) => {
                let size = push_size(op);
                let start = self.pc;
                self.pc = self.pc.saturating_add(size);

                let mut word = [0u8; 32];
                for i in 0..size {
                    let idx = start + i;
                    if idx < self.code.len() {
                        word[32 - size + i] = self.code[idx];
                    }
                }
                self.push_stack(U256::from_big_endian(&word))?;
                Ok(None)
            }
            0x50 => {
                let _ = self.pop_stack()?;
                Ok(None)
            }
            op if is_dup(op) => {
                let depth = dup_index(op);
                self.stack.dup(depth).map_err(Self::map_stack_error)?;
                Ok(None)
            }
            op if is_swap(op) => {
                let depth = swap_index(op);
                self.stack.swap(depth).map_err(Self::map_stack_error)?;
                Ok(None)
            }

            0x01 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b.overflowing_add(a).0)?;
                Ok(None)
            }
            0x02 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b.overflowing_mul(a).0)?;
                Ok(None)
            }
            0x03 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b.overflowing_sub(a).0)?;
                Ok(None)
            }
            0x04 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                if a.is_zero() {
                    self.push_stack(U256::zero())?;
                } else {
                    self.push_stack(b / a)?;
                }
                Ok(None)
            }
            0x05 => {
                let divisor = self.pop_stack()?;
                let dividend = self.pop_stack()?;
                self.push_stack(sdiv(dividend, divisor))?;
                Ok(None)
            }
            0x06 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                if a.is_zero() {
                    self.push_stack(U256::zero())?;
                } else {
                    self.push_stack(b % a)?;
                }
                Ok(None)
            }
            0x07 => {
                let divisor = self.pop_stack()?;
                let dividend = self.pop_stack()?;
                self.push_stack(smod(dividend, divisor))?;
                Ok(None)
            }
            0x08 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                let n = self.pop_stack()?;
                if n.is_zero() {
                    self.push_stack(U256::zero())?;
                } else {
                    let result = (U512::from(a) + U512::from(b)) % U512::from(n);
                    self.push_stack(u512_to_u256(result))?;
                }
                Ok(None)
            }
            0x09 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                let n = self.pop_stack()?;
                if n.is_zero() {
                    self.push_stack(U256::zero())?;
                } else {
                    let result = (U512::from(a) * U512::from(b)) % U512::from(n);
                    self.push_stack(u512_to_u256(result))?;
                }
                Ok(None)
            }
            0x0a => {
                let exponent = self.pop_stack()?;
                let base = self.pop_stack()?;
                let exp_bytes = exponent_byte_size(exponent);
                if exp_bytes > 0 {
                    self.charge_gas(G_EXPBYTE.saturating_mul(exp_bytes as u64))?;
                }
                self.push_stack(exp(base, exponent))?;
                Ok(None)
            }
            0x0b => {
                let byte_index = self.pop_stack()?;
                let value = self.pop_stack()?;
                self.push_stack(signextend(byte_index, value))?;
                Ok(None)
            }

            0x10 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(bool_to_u256(b < a))?;
                Ok(None)
            }
            0x11 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(bool_to_u256(b > a))?;
                Ok(None)
            }
            0x12 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(bool_to_u256(signed_cmp(b, a) == Ordering::Less))?;
                Ok(None)
            }
            0x13 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(bool_to_u256(signed_cmp(b, a) == Ordering::Greater))?;
                Ok(None)
            }
            0x14 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(bool_to_u256(a == b))?;
                Ok(None)
            }
            0x15 => {
                let a = self.pop_stack()?;
                self.push_stack(bool_to_u256(a.is_zero()))?;
                Ok(None)
            }
            0x16 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b & a)?;
                Ok(None)
            }
            0x17 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b | a)?;
                Ok(None)
            }
            0x18 => {
                let a = self.pop_stack()?;
                let b = self.pop_stack()?;
                self.push_stack(b ^ a)?;
                Ok(None)
            }
            0x19 => {
                let a = self.pop_stack()?;
                self.push_stack(!a)?;
                Ok(None)
            }
            0x1a => {
                let index = self.pop_stack()?;
                let value = self.pop_stack()?;
                let out = if index >= U256::from(32u8) {
                    U256::zero()
                } else {
                    let shift = (31usize - index.low_u32() as usize) * 8;
                    (value >> shift) & U256::from(0xffu8)
                };
                self.push_stack(out)?;
                Ok(None)
            }
            0x1b => {
                let shift = self.pop_stack()?;
                let value = self.pop_stack()?;
                let out = if shift >= U256::from(256u16) {
                    U256::zero()
                } else {
                    value << shift.low_u32()
                };
                self.push_stack(out)?;
                Ok(None)
            }
            0x1c => {
                let shift = self.pop_stack()?;
                let value = self.pop_stack()?;
                let out = if shift >= U256::from(256u16) {
                    U256::zero()
                } else {
                    value >> shift.low_u32()
                };
                self.push_stack(out)?;
                Ok(None)
            }
            0x1d => {
                let shift = self.pop_stack()?;
                let value = self.pop_stack()?;
                let out = arithmetic_shift_right(value, u256_to_usize(shift));
                self.push_stack(out)?;
                Ok(None)
            }

            0x20 => {
                let offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let memory_cost = self.memory_expansion_cost(offset, size);
                let word_cost = words_for_size(size).saturating_mul(G_SHA3WORD);
                self.charge_gas(memory_cost.saturating_add(word_cost))?;
                let bytes = self.memory.read_slice(offset, size);
                self.push_stack(keccak_u256(&bytes))?;
                Ok(None)
            }

            0x30 => {
                self.push_stack(self.context.address)?;
                Ok(None)
            }
            0x31 => {
                let address = self.pop_stack()?;
                let access_cost = self.account_access_additional_cost(address);
                self.charge_gas(access_cost)?;
                self.push_stack(self.world.balance_of(address))?;
                Ok(None)
            }
            0x32 => {
                self.push_stack(self.env.origin)?;
                Ok(None)
            }
            0x33 => {
                self.push_stack(self.context.caller)?;
                Ok(None)
            }
            0x34 => {
                self.push_stack(self.context.callvalue)?;
                Ok(None)
            }
            0x35 => {
                let offset = self.pop_usize()?;
                let chunk = read_padded(&self.calldata, offset, 32);
                self.push_stack(U256::from_big_endian(&chunk))?;
                Ok(None)
            }
            0x36 => {
                self.push_stack(U256::from(self.calldata.len()))?;
                Ok(None)
            }
            0x37 => {
                let mem_offset = self.pop_usize()?;
                let data_offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost(mem_offset, size)
                    .saturating_add(copy_cost(size));
                self.charge_gas(dynamic)?;
                let chunk = read_padded(&self.calldata, data_offset, size);
                self.memory.write_slice(mem_offset, &chunk);
                Ok(None)
            }
            0x38 => {
                self.push_stack(U256::from(self.code.len()))?;
                Ok(None)
            }
            0x39 => {
                let mem_offset = self.pop_usize()?;
                let code_offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost(mem_offset, size)
                    .saturating_add(copy_cost(size));
                self.charge_gas(dynamic)?;
                let chunk = read_padded(&self.code, code_offset, size);
                self.memory.write_slice(mem_offset, &chunk);
                Ok(None)
            }
            0x3a => {
                self.push_stack(self.env.gas_price)?;
                Ok(None)
            }
            0x3b => {
                let address = self.pop_stack()?;
                let access_cost = self.account_access_additional_cost(address);
                self.charge_gas(access_cost)?;
                let ext_code = self.code_at(address);
                self.push_stack(U256::from(ext_code.len()))?;
                Ok(None)
            }
            0x3c => {
                let address = self.pop_stack()?;
                let access_cost = self.account_access_additional_cost(address);
                self.charge_gas(access_cost)?;
                let mem_offset = self.pop_usize()?;
                let code_offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost(mem_offset, size)
                    .saturating_add(copy_cost(size));
                self.charge_gas(dynamic)?;
                let ext_code = self.code_at(address);
                let chunk = read_padded(&ext_code, code_offset, size);
                self.memory.write_slice(mem_offset, &chunk);
                Ok(None)
            }
            0x3d => {
                self.push_stack(U256::from(self.last_return_data.len()))?;
                Ok(None)
            }
            0x3e => {
                let mem_offset = self.pop_usize()?;
                let data_offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost(mem_offset, size)
                    .saturating_add(copy_cost(size));
                self.charge_gas(dynamic)?;
                let end = data_offset.saturating_add(size);
                if end > self.last_return_data.len() {
                    return Err(ExitStatus::ReturnDataOutOfBounds);
                }
                let chunk = self.last_return_data[data_offset..end].to_vec();
                self.memory.write_slice(mem_offset, &chunk);
                Ok(None)
            }
            0x3f => {
                let address = self.pop_stack()?;
                let access_cost = self.account_access_additional_cost(address);
                self.charge_gas(access_cost)?;
                let ext_code = self.code_at(address);
                if ext_code.is_empty() {
                    self.push_stack(U256::zero())?;
                } else {
                    self.push_stack(keccak_u256(&ext_code))?;
                }
                Ok(None)
            }

            0x40 => {
                let block_num = self.pop_stack()?;
                let value = self
                    .env
                    .block_hashes
                    .get(&block_num)
                    .copied()
                    .unwrap_or_else(U256::zero);
                self.push_stack(value)?;
                Ok(None)
            }
            0x41 => {
                self.push_stack(self.env.coinbase)?;
                Ok(None)
            }
            0x42 => {
                self.push_stack(self.env.timestamp)?;
                Ok(None)
            }
            0x43 => {
                self.push_stack(self.env.block_number)?;
                Ok(None)
            }
            0x44 => {
                self.push_stack(self.env.prevrandao)?;
                Ok(None)
            }
            0x45 => {
                self.push_stack(self.env.block_gas_limit)?;
                Ok(None)
            }
            0x46 => {
                self.push_stack(self.env.chain_id)?;
                Ok(None)
            }
            0x47 => {
                self.push_stack(self.world.balance_of(self.context.address))?;
                Ok(None)
            }
            0x48 => {
                self.push_stack(self.env.base_fee)?;
                Ok(None)
            }

            0x51 => {
                let offset = self.pop_usize()?;
                self.charge_gas(self.memory_expansion_cost(offset, 32))?;
                let value = self.memory.mload(offset);
                self.push_stack(value)?;
                Ok(None)
            }
            0x52 => {
                let offset = self.pop_usize()?;
                let value = self.pop_stack()?;
                self.charge_gas(self.memory_expansion_cost(offset, 32))?;
                self.memory.mstore(offset, value);
                Ok(None)
            }
            0x53 => {
                let offset = self.pop_usize()?;
                let value = self.pop_stack()?;
                self.charge_gas(self.memory_expansion_cost(offset, 1))?;
                self.memory.mstore8(offset, value);
                Ok(None)
            }
            0x54 => {
                let key = self.pop_stack()?;
                let access_cost = self.storage_access_additional_cost(self.context.address, key);
                self.charge_gas(access_cost)?;
                let value = self.world.storage_load(self.context.address, key);
                self.push_stack(value)?;
                Ok(None)
            }
            0x55 => {
                if self.context.is_static {
                    return Err(ExitStatus::StaticModeViolation);
                }
                let key = self.pop_stack()?;
                let value = self.pop_stack()?;
                let sstore_cost = self.sstore_dynamic_gas(self.context.address, key, value)?;
                self.charge_gas(sstore_cost)?;
                self.world.storage_store(self.context.address, key, value);
                Ok(None)
            }

            0x56 => {
                let dest = self.pop_usize()?;
                if !self.jumpdests.contains(&dest) {
                    return Err(ExitStatus::BadJumpDestination(dest));
                }
                self.pc = dest;
                Ok(None)
            }
            0x57 => {
                let dest = self.pop_usize()?;
                let cond = self.pop_stack()?;
                if !cond.is_zero() {
                    if !self.jumpdests.contains(&dest) {
                        return Err(ExitStatus::BadJumpDestination(dest));
                    }
                    self.pc = dest;
                }
                Ok(None)
            }
            0x58 => {
                self.push_stack(U256::from(pc_before))?;
                Ok(None)
            }
            0x59 => {
                self.push_stack(U256::from(self.memory.len()))?;
                Ok(None)
            }
            0x5a => {
                self.push_stack(U256::from(self.gas_remaining))?;
                Ok(None)
            }
            0x5b => Ok(None),
            0x5e => {
                let dst = self.pop_usize()?;
                let src = self.pop_usize()?;
                let len = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost_two(dst, len, src, len)
                    .saturating_add(copy_cost(len));
                self.charge_gas(dynamic)?;
                self.memory.mcopy(dst, src, len);
                Ok(None)
            }

            op if log_topics(op).is_some() => {
                if self.context.is_static {
                    return Err(ExitStatus::StaticModeViolation);
                }
                let topic_count = log_topics(op).unwrap_or(0);
                let offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                let dynamic = self
                    .memory_expansion_cost(offset, size)
                    .saturating_add((size as u64).saturating_mul(G_LOGDATA));
                self.charge_gas(dynamic)?;
                let mut topics = Vec::with_capacity(topic_count);
                for _ in 0..topic_count {
                    topics.push(self.pop_stack()?);
                }
                let data = self.memory.read_slice(offset, size);
                self.logs.push(LogEntry {
                    address: self.context.address,
                    topics,
                    data,
                });
                Ok(None)
            }

            0xf0 => {
                self.execute_create(false)?;
                Ok(None)
            }
            0xf1 => {
                self.execute_call(CallKind::Call)?;
                Ok(None)
            }
            0xf2 => {
                self.execute_call(CallKind::CallCode)?;
                Ok(None)
            }
            0xf3 => {
                let offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                self.charge_gas(self.memory_expansion_cost(offset, size))?;
                self.return_data = self.memory.read_slice(offset, size);
                Ok(Some(ExitStatus::Return))
            }
            0xf4 => {
                self.execute_call(CallKind::DelegateCall)?;
                Ok(None)
            }
            0xf5 => {
                self.execute_create(true)?;
                Ok(None)
            }
            0xfa => {
                self.execute_call(CallKind::StaticCall)?;
                Ok(None)
            }
            0xfd => {
                let offset = self.pop_usize()?;
                let size = self.pop_usize()?;
                self.charge_gas(self.memory_expansion_cost(offset, size))?;
                self.return_data = self.memory.read_slice(offset, size);
                Ok(Some(ExitStatus::Revert))
            }
            0xfe => Err(ExitStatus::InvalidOpcode(opcode)),
            0xff => {
                if self.context.is_static {
                    return Err(ExitStatus::StaticModeViolation);
                }
                let beneficiary = self.pop_stack()?;
                let access_cost = self.account_access_cost(beneficiary);
                self.charge_gas(access_cost)?;
                self.world.selfdestruct(self.context.address, beneficiary);
                Ok(Some(ExitStatus::SelfDestruct))
            }
            _ => Err(ExitStatus::InvalidOpcode(opcode)),
        }
    }

    fn execute_call(&mut self, kind: CallKind) -> Result<(), ExitStatus> {
        let gas = self.pop_stack()?;
        let to = self.pop_stack()?;

        let (value, in_offset, in_size, out_offset, out_size) = match kind {
            CallKind::Call | CallKind::CallCode => {
                let value = self.pop_stack()?;
                let in_offset = self.pop_usize()?;
                let in_size = self.pop_usize()?;
                let out_offset = self.pop_usize()?;
                let out_size = self.pop_usize()?;
                (value, in_offset, in_size, out_offset, out_size)
            }
            CallKind::DelegateCall | CallKind::StaticCall => {
                let in_offset = self.pop_usize()?;
                let in_size = self.pop_usize()?;
                let out_offset = self.pop_usize()?;
                let out_size = self.pop_usize()?;
                (U256::zero(), in_offset, in_size, out_offset, out_size)
            }
        };

        let access_cost = self.account_access_cost(to);
        self.charge_gas(access_cost)?;

        let memory_cost = self.memory_expansion_cost_two(in_offset, in_size, out_offset, out_size);
        self.charge_gas(memory_cost)?;

        let mut extra_cost = 0u64;
        let caller_address = self.context.address;
        let has_value = !value.is_zero() && matches!(kind, CallKind::Call | CallKind::CallCode);
        if has_value {
            extra_cost = extra_cost.saturating_add(G_CALLVALUE);
        }
        if matches!(kind, CallKind::Call) && !value.is_zero() && !self.world.exists(to) {
            extra_cost = extra_cost.saturating_add(G_NEWACCOUNT);
        }
        self.charge_gas(extra_cost)?;

        if self.context.depth + 1 >= MAX_CALL_DEPTH {
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        if self.context.is_static && matches!(kind, CallKind::Call) && !value.is_zero() {
            return Err(ExitStatus::StaticModeViolation);
        }

        if has_value && self.world.balance_of(caller_address) < value {
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        let (context_address, context_caller, context_callvalue, code_address, child_is_static) =
            match kind {
                CallKind::Call => (to, caller_address, value, to, self.context.is_static),
                CallKind::CallCode => (
                    caller_address,
                    caller_address,
                    value,
                    to,
                    self.context.is_static,
                ),
                CallKind::DelegateCall => (
                    caller_address,
                    self.context.caller,
                    self.context.callvalue,
                    to,
                    self.context.is_static,
                ),
                CallKind::StaticCall => (to, caller_address, U256::zero(), to, true),
            };

        let requested_gas = u256_to_u64(gas);
        let gas_cap = max_call_gas(self.gas_remaining);
        let mut forwarded_gas = requested_gas.min(gas_cap);
        if has_value {
            forwarded_gas = forwarded_gas.saturating_add(G_CALLSTIPEND);
        }

        let snapshot = self.world.clone();
        let should_transfer = matches!(kind, CallKind::Call) && !value.is_zero();
        if should_transfer && !self.world.transfer(caller_address, to, value) {
            self.world = snapshot;
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        self.charge_gas(forwarded_gas)?;

        let snapshot_accessed_addresses = self.accessed_addresses.clone();
        let snapshot_accessed_storage_keys = self.accessed_storage_keys.clone();
        let snapshot_refund = self.gas_refund;

        let input = self.memory.read_slice(in_offset, in_size);
        let child_context = FrameContext {
            address: context_address,
            caller: context_caller,
            callvalue: context_callvalue,
            is_static: child_is_static,
            depth: self.context.depth + 1,
        };
        let child_code = self.code_at(code_address);
        let child_world = self.world.clone();
        let mut child =
            self.spawn_child(child_code, input, forwarded_gas, child_context, child_world);
        let child_result = child.run();
        self.gas_remaining = self
            .gas_remaining
            .saturating_add(child_result.gas_remaining);

        let success = is_success_status(&child_result.status);
        self.merge_original_storage(&child.original_storage);
        if success {
            self.world = child.world;
            self.accessed_addresses = child.accessed_addresses;
            self.accessed_storage_keys = child.accessed_storage_keys;
            self.gas_refund = child.gas_refund;
            self.logs.extend(child_result.logs.clone());
        } else {
            self.world = snapshot;
            self.accessed_addresses = snapshot_accessed_addresses;
            self.accessed_storage_keys = snapshot_accessed_storage_keys;
            self.gas_refund = snapshot_refund;
        }

        self.last_return_data = match child_result.status {
            ExitStatus::Return | ExitStatus::Revert => child_result.return_data.clone(),
            _ => Vec::new(),
        };
        self.copy_last_return_data_to_memory(out_offset, out_size);

        self.push_stack(bool_to_u256(success))?;
        Ok(())
    }

    fn execute_create(&mut self, is_create2: bool) -> Result<(), ExitStatus> {
        if self.context.is_static {
            return Err(ExitStatus::StaticModeViolation);
        }

        let value = self.pop_stack()?;
        let offset = self.pop_usize()?;
        let size = self.pop_usize()?;
        let salt = if is_create2 {
            Some(self.pop_stack()?)
        } else {
            None
        };

        if self.context.depth + 1 >= MAX_CALL_DEPTH {
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        let memory_cost = self.memory_expansion_cost(offset, size);
        let create2_hash_cost = if is_create2 {
            words_for_size(size).saturating_mul(G_SHA3WORD)
        } else {
            0
        };
        let initcode_word_cost = words_for_size(size).saturating_mul(G_INITCODE_WORD);
        self.charge_gas(
            memory_cost
                .saturating_add(create2_hash_cost)
                .saturating_add(initcode_word_cost),
        )?;

        if size > MAX_INITCODE_SIZE {
            return Err(ExitStatus::OutOfGas);
        }

        let init_code = self.memory.read_slice(offset, size);
        let creator = self.context.address;
        if !value.is_zero() && self.world.balance_of(creator) < value {
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        let snapshot = self.world.clone();
        let nonce = self.world.increment_nonce(creator);

        let new_address = if let Some(salt_value) = salt {
            create2_address(creator, salt_value, &init_code)
        } else {
            create_address(creator, nonce)
        };
        self.accessed_addresses.insert(new_address);

        if !self.code_at(new_address).is_empty() {
            self.world = snapshot;
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        if !value.is_zero() && !self.world.transfer(creator, new_address, value) {
            self.world = snapshot;
            self.last_return_data.clear();
            self.push_stack(U256::zero())?;
            return Ok(());
        }

        let forwarded_gas = max_call_gas(self.gas_remaining);
        self.charge_gas(forwarded_gas)?;

        let snapshot_accessed_addresses = self.accessed_addresses.clone();
        let snapshot_accessed_storage_keys = self.accessed_storage_keys.clone();
        let snapshot_refund = self.gas_refund;

        let child_context = FrameContext {
            address: new_address,
            caller: creator,
            callvalue: value,
            is_static: false,
            depth: self.context.depth + 1,
        };
        let child_world = self.world.clone();
        let mut child = self.spawn_child(
            init_code,
            Vec::new(),
            forwarded_gas,
            child_context,
            child_world,
        );
        let child_result = child.run();
        self.gas_remaining = self
            .gas_remaining
            .saturating_add(child_result.gas_remaining);

        let success = is_success_status(&child_result.status);
        self.merge_original_storage(&child.original_storage);
        if success {
            let runtime_code = if matches!(child_result.status, ExitStatus::Return) {
                child_result.return_data.clone()
            } else {
                Vec::new()
            };
            let code_deposit_cost =
                (runtime_code.len() as u64).saturating_mul(G_CREATE_CODE_DEPOSIT);
            if self.gas_remaining < code_deposit_cost {
                self.world = snapshot;
                self.accessed_addresses = snapshot_accessed_addresses;
                self.accessed_storage_keys = snapshot_accessed_storage_keys;
                self.gas_refund = snapshot_refund;
                self.gas_remaining = 0;
                self.last_return_data.clear();
                self.push_stack(U256::zero())?;
                return Ok(());
            }

            self.charge_gas(code_deposit_cost)?;
            self.world = child.world;
            self.accessed_addresses = child.accessed_addresses;
            self.accessed_storage_keys = child.accessed_storage_keys;
            self.gas_refund = child.gas_refund;
            let runtime_code = if matches!(child_result.status, ExitStatus::Return) {
                child_result.return_data.clone()
            } else {
                Vec::new()
            };
            self.world.deploy_code(new_address, runtime_code);
            self.logs.extend(child_result.logs.clone());
            self.last_return_data.clear();
            self.push_stack(new_address)?;
        } else {
            self.world = snapshot;
            self.accessed_addresses = snapshot_accessed_addresses;
            self.accessed_storage_keys = snapshot_accessed_storage_keys;
            self.gas_refund = snapshot_refund;
            self.last_return_data = if matches!(child_result.status, ExitStatus::Revert) {
                child_result.return_data.clone()
            } else {
                Vec::new()
            };
            self.push_stack(U256::zero())?;
        }

        Ok(())
    }

    fn spawn_child(
        &self,
        code: Vec<u8>,
        calldata: Vec<u8>,
        gas_limit: u64,
        context: FrameContext,
        world: WorldState,
    ) -> Evm {
        Evm {
            code: code.clone(),
            pc: 0,
            stack: Stack::new(),
            memory: Memory::new(),
            calldata,
            gas_limit,
            gas_remaining: gas_limit,
            return_data: Vec::new(),
            last_return_data: Vec::new(),
            jumpdests: collect_jumpdests(&code),
            env: self.env.clone(),
            world,
            original_storage: self.original_storage.clone(),
            accessed_addresses: self.accessed_addresses.clone(),
            accessed_storage_keys: self.accessed_storage_keys.clone(),
            gas_refund: self.gas_refund,
            logs: Vec::new(),
            trace: self.trace,
            context,
        }
    }

    fn copy_last_return_data_to_memory(&mut self, out_offset: usize, out_size: usize) {
        if out_size == 0 || self.last_return_data.is_empty() {
            return;
        }
        let copy_len = out_size.min(self.last_return_data.len());
        self.memory
            .write_slice(out_offset, &self.last_return_data[..copy_len]);
    }

    fn merge_original_storage(&mut self, child_original_storage: &HashMap<(U256, U256), U256>) {
        for (key, value) in child_original_storage {
            self.original_storage.entry(*key).or_insert(*value);
        }
    }

    fn reset_access_lists(&mut self) {
        self.accessed_addresses.clear();
        self.accessed_storage_keys.clear();
        self.gas_refund = 0;
        self.original_storage.clear();

        self.accessed_addresses.insert(self.context.address);
        self.accessed_addresses.insert(self.context.caller);
        self.accessed_addresses.insert(self.env.origin);

        for precompile in 1u8..=9u8 {
            self.accessed_addresses.insert(U256::from(precompile));
        }
    }

    fn account_access_cost(&mut self, address: U256) -> u64 {
        if self.accessed_addresses.insert(address) {
            COLD_ACCOUNT_ACCESS_COST
        } else {
            WARM_STORAGE_READ_COST
        }
    }

    fn account_access_additional_cost(&mut self, address: U256) -> u64 {
        self.account_access_cost(address)
            .saturating_sub(WARM_STORAGE_READ_COST)
    }

    fn storage_access_cost(&mut self, address: U256, key: U256) -> u64 {
        if self.accessed_storage_keys.insert((address, key)) {
            COLD_SLOAD_COST
        } else {
            WARM_STORAGE_READ_COST
        }
    }

    fn storage_access_additional_cost(&mut self, address: U256, key: U256) -> u64 {
        self.storage_access_cost(address, key)
            .saturating_sub(WARM_STORAGE_READ_COST)
    }

    fn charge_gas(&mut self, amount: u64) -> Result<(), ExitStatus> {
        if self.gas_remaining < amount {
            return Err(ExitStatus::OutOfGas);
        }
        self.gas_remaining -= amount;
        Ok(())
    }

    fn memory_word_size(&self) -> u64 {
        words_for_size(self.memory.len())
    }

    fn memory_expansion_cost(&self, offset: usize, size: usize) -> u64 {
        self.memory_expansion_cost_many(&[(offset, size)])
    }

    fn memory_expansion_cost_two(
        &self,
        first_offset: usize,
        first_size: usize,
        second_offset: usize,
        second_size: usize,
    ) -> u64 {
        self.memory_expansion_cost_many(&[(first_offset, first_size), (second_offset, second_size)])
    }

    fn memory_expansion_cost_many(&self, ranges: &[(usize, usize)]) -> u64 {
        let current_words = self.memory_word_size();
        let mut new_words = current_words;
        for (offset, size) in ranges {
            let words = words_for_range(*offset, *size);
            if words > new_words {
                new_words = words;
            }
        }
        if new_words <= current_words {
            return 0;
        }
        memory_cost(new_words).saturating_sub(memory_cost(current_words))
    }

    fn sstore_dynamic_gas(
        &mut self,
        address: U256,
        slot: U256,
        new_value: U256,
    ) -> Result<u64, ExitStatus> {
        if self.gas_remaining <= SSTORE_STIPEND {
            return Err(ExitStatus::OutOfGas);
        }

        let is_cold = self.storage_access_cost(address, slot) == COLD_SLOAD_COST;

        let original_value = *self
            .original_storage
            .entry((address, slot))
            .or_insert_with(|| self.world.storage_load(address, slot));
        let current_value = self.world.storage_load(address, slot);

        let mut gas_cost: u64;
        if new_value == current_value {
            gas_cost = SLOAD_GAS;
        } else if original_value == current_value {
            if original_value.is_zero() {
                gas_cost = SSTORE_SET_GAS;
            } else {
                gas_cost = SSTORE_RESET_GAS;
                if new_value.is_zero() {
                    self.gas_refund += SSTORE_CLEARS_SCHEDULE;
                }
            }
        } else {
            gas_cost = SLOAD_GAS;

            if !original_value.is_zero() {
                if current_value.is_zero() {
                    self.gas_refund -= SSTORE_CLEARS_SCHEDULE;
                }
                if new_value.is_zero() {
                    self.gas_refund += SSTORE_CLEARS_SCHEDULE;
                }
            }

            if new_value == original_value {
                if original_value.is_zero() {
                    self.gas_refund += (SSTORE_SET_GAS - SLOAD_GAS) as i64;
                } else {
                    self.gas_refund += (SSTORE_RESET_GAS - SLOAD_GAS) as i64;
                }
            }
        }

        if is_cold {
            gas_cost = gas_cost.saturating_add(COLD_SLOAD_COST);
        }

        Ok(gas_cost)
    }

    fn push_stack(&mut self, value: U256) -> Result<(), ExitStatus> {
        self.stack.push(value).map_err(Self::map_stack_error)
    }

    fn pop_stack(&mut self) -> Result<U256, ExitStatus> {
        self.stack.pop().map_err(Self::map_stack_error)
    }

    fn pop_usize(&mut self) -> Result<usize, ExitStatus> {
        let value = self.pop_stack()?;
        Ok(u256_to_usize(value))
    }

    fn code_at(&self, address: U256) -> Vec<u8> {
        self.world.code_of(address)
    }

    fn map_stack_error(err: StackError) -> ExitStatus {
        match err {
            StackError::Overflow => ExitStatus::StackOverflow,
            StackError::Underflow | StackError::InvalidDepth => ExitStatus::StackUnderflow,
        }
    }

    fn print_trace(&self, pc: usize, opcode: u8) {
        let stack = self
            .stack
            .as_slice()
            .iter()
            .map(|v| format_u256(*v))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "depth={} pc={pc:04x} op={} gas_remaining={} stack=[{stack}]",
            self.context.depth,
            opcode_name(opcode),
            self.gas_remaining
        );
    }
}

pub fn format_u256(value: U256) -> String {
    if value.is_zero() {
        return "0x0".to_string();
    }
    let bytes = value.to_big_endian();
    let encoded = hex::encode(bytes);
    let trimmed = encoded.trim_start_matches('0');
    format!("0x{trimmed}")
}

pub fn format_stack(stack: &[U256]) -> String {
    let formatted = stack
        .iter()
        .map(|v| format_u256(*v))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{formatted}]")
}

fn collect_jumpdests(code: &[u8]) -> HashSet<usize> {
    let mut destinations = HashSet::new();
    let mut pc = 0usize;
    while pc < code.len() {
        let opcode = code[pc];
        if opcode == 0x5b {
            destinations.insert(pc);
        }
        if is_push(opcode) {
            pc = pc.saturating_add(1 + push_size(opcode));
        } else {
            pc = pc.saturating_add(1);
        }
    }
    destinations
}

fn read_padded(source: &[u8], offset: usize, size: usize) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; size];
    if offset >= source.len() {
        return out;
    }
    let available = source.len() - offset;
    let to_copy = available.min(size);
    out[..to_copy].copy_from_slice(&source[offset..offset + to_copy]);
    out
}

fn words_for_size(size: usize) -> u64 {
    if size == 0 {
        return 0;
    }
    ((size as u64).saturating_add(31)) / 32
}

fn words_for_range(offset: usize, size: usize) -> u64 {
    if size == 0 {
        return 0;
    }
    let end = offset.saturating_add(size);
    words_for_size(end)
}

fn memory_cost(words: u64) -> u64 {
    G_MEMORY
        .saturating_mul(words)
        .saturating_add(words.saturating_mul(words) / 512)
}

fn copy_cost(size: usize) -> u64 {
    words_for_size(size).saturating_mul(G_COPY)
}

fn max_call_gas(gas: u64) -> u64 {
    gas.saturating_sub(gas / 64)
}

fn keccak_u256(data: &[u8]) -> U256 {
    let digest = keccak256(data);
    U256::from_big_endian(&digest)
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut digest = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut digest);
    digest
}

fn create_address(creator: U256, nonce: u64) -> U256 {
    let creator = address20(creator);
    let encoded_sender = rlp_encode_bytes(&creator);
    let encoded_nonce = rlp_encode_u64(nonce);
    let list = rlp_encode_list(&[encoded_sender, encoded_nonce]);
    let digest = keccak256(&list);
    u256_from_address20(&digest[12..32])
}

fn create2_address(creator: U256, salt: U256, init_code: &[u8]) -> U256 {
    let creator = address20(creator);
    let init_hash = keccak256(init_code);
    let salt_bytes = salt.to_big_endian();

    let mut payload = Vec::with_capacity(1 + 20 + 32 + 32);
    payload.push(0xff);
    payload.extend_from_slice(&creator);
    payload.extend_from_slice(&salt_bytes);
    payload.extend_from_slice(&init_hash);
    let digest = keccak256(&payload);
    u256_from_address20(&digest[12..32])
}

fn address20(value: U256) -> [u8; 20] {
    let full = value.to_big_endian();
    let mut out = [0u8; 20];
    out.copy_from_slice(&full[12..32]);
    out
}

fn u256_from_address20(bytes: &[u8]) -> U256 {
    let mut full = [0u8; 32];
    full[12..32].copy_from_slice(bytes);
    U256::from_big_endian(&full)
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }
    if bytes.len() <= 55 {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        return out;
    }

    let len_bytes = trim_be(&(bytes.len() as u64).to_be_bytes());
    let mut out = Vec::with_capacity(1 + len_bytes.len() + bytes.len());
    out.push(0xb7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(bytes);
    out
}

fn rlp_encode_u64(value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![0x80];
    }
    let be = trim_be(&value.to_be_bytes());
    rlp_encode_bytes(&be)
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload_len: usize = items.iter().map(Vec::len).sum();
    if payload_len <= 55 {
        let mut out = Vec::with_capacity(1 + payload_len);
        out.push(0xc0 + payload_len as u8);
        for item in items {
            out.extend_from_slice(item);
        }
        return out;
    }

    let len_bytes = trim_be(&(payload_len as u64).to_be_bytes());
    let mut out = Vec::with_capacity(1 + len_bytes.len() + payload_len);
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn trim_be(bytes: &[u8]) -> Vec<u8> {
    let first_non_zero = bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(bytes.len() - 1);
    bytes[first_non_zero..].to_vec()
}

fn bool_to_u256(value: bool) -> U256 {
    if value { U256::one() } else { U256::zero() }
}

fn is_success_status(status: &ExitStatus) -> bool {
    matches!(
        status,
        ExitStatus::Stop | ExitStatus::Return | ExitStatus::SelfDestruct
    )
}

fn u512_to_u256(value: U512) -> U256 {
    let bytes = value.to_big_endian();
    U256::from_big_endian(&bytes[32..])
}

fn u256_to_u64(value: U256) -> u64 {
    let max = U256::from(u64::MAX);
    if value > max {
        u64::MAX
    } else {
        value.low_u64()
    }
}

fn u256_to_usize(value: U256) -> usize {
    let max = U256::from(usize::MAX as u64);
    if value > max {
        usize::MAX
    } else {
        value.low_u64() as usize
    }
}

fn is_negative(value: U256) -> bool {
    value.bit(255)
}

fn twos_complement(value: U256) -> U256 {
    (!value).overflowing_add(U256::one()).0
}

fn signed_abs(value: U256) -> U256 {
    if is_negative(value) {
        twos_complement(value)
    } else {
        value
    }
}

fn signed_cmp(lhs: U256, rhs: U256) -> Ordering {
    let lhs_neg = is_negative(lhs);
    let rhs_neg = is_negative(rhs);
    match (lhs_neg, rhs_neg) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => lhs.cmp(&rhs),
        (true, true) => signed_abs(rhs).cmp(&signed_abs(lhs)),
    }
}

fn sdiv(dividend: U256, divisor: U256) -> U256 {
    if divisor.is_zero() {
        return U256::zero();
    }

    let result_negative = is_negative(dividend) ^ is_negative(divisor);
    let abs_dividend = signed_abs(dividend);
    let abs_divisor = signed_abs(divisor);
    let quotient = abs_dividend / abs_divisor;

    if quotient.is_zero() {
        U256::zero()
    } else if result_negative {
        twos_complement(quotient)
    } else {
        quotient
    }
}

fn smod(dividend: U256, divisor: U256) -> U256 {
    if divisor.is_zero() {
        return U256::zero();
    }

    let abs_dividend = signed_abs(dividend);
    let abs_divisor = signed_abs(divisor);
    let remainder = abs_dividend % abs_divisor;

    if remainder.is_zero() {
        U256::zero()
    } else if is_negative(dividend) {
        twos_complement(remainder)
    } else {
        remainder
    }
}

fn exp(base: U256, exponent: U256) -> U256 {
    let mut result = U256::one();
    let mut running_base = base;
    let mut e = exponent;
    while !e.is_zero() {
        if e.bit(0) {
            result = result.overflowing_mul(running_base).0;
        }
        e >>= 1;
        if !e.is_zero() {
            running_base = running_base.overflowing_mul(running_base).0;
        }
    }
    result
}

fn exponent_byte_size(exponent: U256) -> usize {
    if exponent.is_zero() {
        return 0;
    }
    let bytes = exponent.to_big_endian();
    let first_non_zero = bytes.iter().position(|b| *b != 0).unwrap_or(31);
    32 - first_non_zero
}

fn signextend(byte_index: U256, value: U256) -> U256 {
    if byte_index >= U256::from(32u8) {
        return value;
    }
    let bit_index = (byte_index.low_u32() as usize) * 8 + 7;
    if bit_index >= 255 {
        return value;
    }

    if value.bit(bit_index) {
        let mask = U256::MAX << (bit_index + 1);
        value | mask
    } else {
        let mask = (U256::one() << (bit_index + 1)) - U256::one();
        value & mask
    }
}

fn arithmetic_shift_right(value: U256, shift: usize) -> U256 {
    if shift == 0 {
        return value;
    }
    if shift >= 256 {
        return if is_negative(value) {
            U256::MAX
        } else {
            U256::zero()
        };
    }
    if !is_negative(value) {
        return value >> shift;
    }
    let shifted = value >> shift;
    let fill_mask = U256::MAX << (256 - shift);
    shifted | fill_mask
}
