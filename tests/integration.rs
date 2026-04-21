use evm::evm::{Evm, ExitStatus};
use primitive_types::U256;

fn run(code: Vec<u8>) -> evm::evm::ExecutionResult {
    let mut vm = Evm::new(code).with_gas_limit(5_000_000);
    vm.run()
}

fn run_with_calldata(code: Vec<u8>, calldata: Vec<u8>) -> evm::evm::ExecutionResult {
    let mut vm = Evm::new(code)
        .with_gas_limit(5_000_000)
        .with_calldata(calldata);
    vm.run()
}

fn twos_complement_negative(n: u64) -> U256 {
    (!U256::from(n)).overflowing_add(U256::one()).0
}

#[test]
fn simple_add_program() {
    let code = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack.len(), 1);
    assert_eq!(result.stack[0], U256::from(3u8));
}

#[test]
fn stack_overflow_is_reported() {
    let mut code = Vec::new();
    for _ in 0..1025 {
        code.extend([0x60, 0x00]);
    }
    let result = run(code);
    assert_eq!(result.status, ExitStatus::StackOverflow);
}

#[test]
fn integer_overflow_wraps() {
    let mut code = vec![0x7f];
    code.extend([0xff; 32]);
    code.extend([0x60, 0x01, 0x01, 0x00]);
    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack[0], U256::zero());
}

#[test]
fn gas_exhaustion_is_reported() {
    let code = vec![0x60, 0x01, 0x00];
    let mut vm = Evm::new(code).with_gas_limit(2);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn invalid_jump_destination_fails() {
    let code = vec![0x60, 0x02, 0x56, 0x00];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::BadJumpDestination(2));
}

#[test]
fn revert_returns_data() {
    let code = vec![0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::Revert);
    assert_eq!(result.return_data.len(), 32);
    assert_eq!(result.return_data[31], 0x2a);
}

#[test]
fn multi_opcode_program_executes() {
    let code = vec![
        0x60, 0x02, 0x60, 0x03, 0x01, // 2 + 3
        0x60, 0x04, 0x02, // * 4
        0x60, 0x05, 0x03, // - 5
        0x00,
    ];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack[0], U256::from(15u8));
}

#[test]
fn jumpi_to_valid_jumpdest() {
    let code = vec![0x60, 0x01, 0x60, 0x06, 0x57, 0x00, 0x5b, 0x60, 0x2a, 0x00];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack[0], U256::from(0x2au8));
}

#[test]
fn sdiv_handles_negative_values() {
    let mut code = vec![0x7f];
    let mut neg_ten = [0u8; 32];
    twos_complement_negative(10).to_big_endian(&mut neg_ten);
    code.extend(neg_ten);
    code.extend([0x60, 0x03, 0x05, 0x00]);

    let result = run(code);
    let expected = twos_complement_negative(3);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack[0], expected);
}

#[test]
fn sar_sign_extends() {
    let mut code = vec![0x7f];
    let mut neg_two = [0u8; 32];
    twos_complement_negative(2).to_big_endian(&mut neg_two);
    code.extend(neg_two);
    code.extend([0x60, 0x01, 0x1d, 0x00]);

    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack[0], U256::MAX);
}

#[test]
fn calldata_copy_and_mload_work() {
    let code = vec![
        0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0x37, 0x60, 0x00, 0x51, 0x00,
    ];
    let result = run_with_calldata(code, vec![0xaa, 0xbb, 0xcc]);
    assert_eq!(result.status, ExitStatus::Stop);
    let mut bytes = [0u8; 32];
    result.stack[0].to_big_endian(&mut bytes);
    assert_eq!(&bytes[0..3], &[0xaa, 0xbb, 0xcc]);
}

#[test]
fn returndatacopy_out_of_bounds_fails() {
    let code = vec![0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0x3e, 0x00];
    let result = run(code);
    assert_eq!(result.status, ExitStatus::ReturnDataOutOfBounds);
}

#[test]
fn call_executes_external_contract_code() {
    let callee = vec![0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];
    let caller = vec![
        0x60, 0x20, // out size
        0x60, 0x00, // out offset
        0x60, 0x00, // in size
        0x60, 0x00, // in offset
        0x60, 0x00, // value
        0x60, 0x01, // to
        0x61, 0x03, 0xe8, // gas
        0xf1, // CALL
        0x60, 0x00, 0x51, // MLOAD(0)
        0x00,
    ];

    let mut vm = Evm::new(caller).with_external_code(U256::from(1u8), callee);
    let result = vm.run();

    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack.len(), 2);
    assert_eq!(result.stack[0], U256::one()); // CALL success
    assert_eq!(result.stack[1], U256::from(0x2au8)); // returndata copied to memory
}

#[test]
fn delegatecall_uses_caller_storage_context() {
    let callee = vec![
        0x60, 0x00, 0x54, // SLOAD(0)
        0x60, 0x01, 0x01, // +1
        0x60, 0x00, 0x55, // SSTORE(0)
        0x00,
    ];
    let caller = vec![
        0x60, 0x00, // out size
        0x60, 0x00, // out offset
        0x60, 0x00, // in size
        0x60, 0x00, // in offset
        0x60, 0x01, // to
        0x61, 0x75, 0x30, // gas
        0xf4, // DELEGATECALL
        0x60, 0x00, 0x54, // SLOAD(0) in caller context
        0x00,
    ];

    let mut vm = Evm::new(caller).with_external_code(U256::from(1u8), callee);
    let result = vm.run();

    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack.len(), 2);
    assert_eq!(result.stack[0], U256::one()); // DELEGATECALL success
    assert_eq!(result.stack[1], U256::one()); // slot 0 incremented in caller storage
}

#[test]
fn create_deploys_runtime_code() {
    let mut code = vec![0x69];
    code.extend([0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]); // init code
    code.extend([
        0x60, 0x00, 0x52, // MSTORE(0)
        0x60, 0x0a, // size 10
        0x60, 0x16, // offset 22 (PUSH10 right-aligned in word)
        0x60, 0x00, // value 0
        0xf0, // CREATE
        0x80, // DUP1(created address)
        0x3b, // EXTCODESIZE(address)
        0x00,
    ]);

    let result = run(code);
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack.len(), 2);
    assert_ne!(result.stack[0], U256::zero());
    assert_eq!(result.stack[1], U256::from(32u8)); // init code returned 32 bytes as runtime
}

#[test]
fn call_revert_sets_returndata() {
    let callee = vec![0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd];
    let caller = vec![
        0x60, 0x00, // out size
        0x60, 0x00, // out offset
        0x60, 0x00, // in size
        0x60, 0x00, // in offset
        0x60, 0x00, // value
        0x60, 0x01, // to
        0x61, 0x03, 0xe8, // gas
        0xf1, // CALL
        0x3d, // RETURNDATASIZE
        0x00,
    ];

    let mut vm = Evm::new(caller).with_external_code(U256::from(1u8), callee);
    let result = vm.run();

    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.stack.len(), 2);
    assert_eq!(result.stack[0], U256::zero()); // CALL failed because callee REVERTed
    assert_eq!(result.stack[1], U256::from(32u8)); // returndata still available
}

#[test]
fn invalid_opcode_consumes_all_gas() {
    let mut vm = Evm::new(vec![0xfe]).with_gas_limit(100);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::InvalidOpcode(0xfe));
    assert_eq!(result.gas_remaining, 0);
}

#[test]
fn exp_charges_dynamic_gas_by_exponent_size() {
    let code = vec![0x60, 0x02, 0x61, 0x01, 0x00, 0x0a, 0x00];
    let mut vm = Evm::new(code).with_gas_limit(115);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn calldatacopy_charges_memory_and_copy_gas() {
    let code = vec![0x60, 0x40, 0x60, 0x00, 0x60, 0x00, 0x37, 0x00];
    let mut vm = Evm::new(code).with_gas_limit(23);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn sstore_dynamic_gas_is_enforced() {
    let code = vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00];
    let mut vm = Evm::new(code).with_gas_limit(20_005);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn balance_uses_cold_then_warm_access_cost() {
    let code = vec![
        0x60, 0x0a, // address (not precompile)
        0x31, // BALANCE (cold)
        0x50, // POP
        0x60, 0x0a, // address
        0x31, // BALANCE (warm)
        0x50, // POP
        0x00,
    ];
    let mut vm = Evm::new(code).with_gas_limit(2_709);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn sload_uses_cold_then_warm_slot_cost() {
    let code = vec![
        0x60, 0x00, // slot
        0x54, // SLOAD (cold)
        0x50, // POP
        0x60, 0x00, // slot
        0x54, // SLOAD (warm)
        0x00,
    ];
    let mut vm = Evm::new(code).with_gas_limit(2_207);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::OutOfGas);
}

#[test]
fn sstore_refund_is_capped_at_one_fifth() {
    let code = vec![
        0x60, 0x01, 0x60, 0x00, 0x55, // set slot 0 = 1
        0x60, 0x00, 0x60, 0x00, 0x55, // clear slot 0 = 0
        0x00,
    ];
    let mut vm = Evm::new(code).with_gas_limit(30_000);
    let result = vm.run();
    assert_eq!(result.status, ExitStatus::Stop);
    assert_eq!(result.gas_used, 17_770);
}
