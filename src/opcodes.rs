pub fn is_push(opcode: u8) -> bool {
    (0x60..=0x7f).contains(&opcode)
}

pub fn push_size(opcode: u8) -> usize {
    (opcode - 0x5f) as usize
}

pub fn is_dup(opcode: u8) -> bool {
    (0x80..=0x8f).contains(&opcode)
}

pub fn dup_index(opcode: u8) -> usize {
    (opcode - 0x7f) as usize
}

pub fn is_swap(opcode: u8) -> bool {
    (0x90..=0x9f).contains(&opcode)
}

pub fn swap_index(opcode: u8) -> usize {
    (opcode - 0x8f) as usize
}

pub fn log_topics(opcode: u8) -> Option<usize> {
    match opcode {
        0xa0 => Some(0),
        0xa1 => Some(1),
        0xa2 => Some(2),
        0xa3 => Some(3),
        0xa4 => Some(4),
        _ => None,
    }
}

pub fn opcode_name(opcode: u8) -> String {
    match opcode {
        0x00 => "STOP".to_string(),
        0x01 => "ADD".to_string(),
        0x02 => "MUL".to_string(),
        0x03 => "SUB".to_string(),
        0x04 => "DIV".to_string(),
        0x05 => "SDIV".to_string(),
        0x06 => "MOD".to_string(),
        0x07 => "SMOD".to_string(),
        0x08 => "ADDMOD".to_string(),
        0x09 => "MULMOD".to_string(),
        0x0a => "EXP".to_string(),
        0x0b => "SIGNEXTEND".to_string(),
        0x10 => "LT".to_string(),
        0x11 => "GT".to_string(),
        0x12 => "SLT".to_string(),
        0x13 => "SGT".to_string(),
        0x14 => "EQ".to_string(),
        0x15 => "ISZERO".to_string(),
        0x16 => "AND".to_string(),
        0x17 => "OR".to_string(),
        0x18 => "XOR".to_string(),
        0x19 => "NOT".to_string(),
        0x1a => "BYTE".to_string(),
        0x1b => "SHL".to_string(),
        0x1c => "SHR".to_string(),
        0x1d => "SAR".to_string(),
        0x20 => "SHA3".to_string(),
        0x30 => "ADDRESS".to_string(),
        0x31 => "BALANCE".to_string(),
        0x32 => "ORIGIN".to_string(),
        0x33 => "CALLER".to_string(),
        0x34 => "CALLVALUE".to_string(),
        0x35 => "CALLDATALOAD".to_string(),
        0x36 => "CALLDATASIZE".to_string(),
        0x37 => "CALLDATACOPY".to_string(),
        0x38 => "CODESIZE".to_string(),
        0x39 => "CODECOPY".to_string(),
        0x3a => "GASPRICE".to_string(),
        0x3b => "EXTCODESIZE".to_string(),
        0x3c => "EXTCODECOPY".to_string(),
        0x3d => "RETURNDATASIZE".to_string(),
        0x3e => "RETURNDATACOPY".to_string(),
        0x3f => "EXTCODEHASH".to_string(),
        0x40 => "BLOCKHASH".to_string(),
        0x41 => "COINBASE".to_string(),
        0x42 => "TIMESTAMP".to_string(),
        0x43 => "NUMBER".to_string(),
        0x44 => "PREVRANDAO".to_string(),
        0x45 => "GASLIMIT".to_string(),
        0x46 => "CHAINID".to_string(),
        0x47 => "SELFBALANCE".to_string(),
        0x48 => "BASEFEE".to_string(),
        0x50 => "POP".to_string(),
        0x51 => "MLOAD".to_string(),
        0x52 => "MSTORE".to_string(),
        0x53 => "MSTORE8".to_string(),
        0x54 => "SLOAD".to_string(),
        0x55 => "SSTORE".to_string(),
        0x56 => "JUMP".to_string(),
        0x57 => "JUMPI".to_string(),
        0x58 => "PC".to_string(),
        0x59 => "MSIZE".to_string(),
        0x5a => "GAS".to_string(),
        0x5b => "JUMPDEST".to_string(),
        0x5e => "MCOPY".to_string(),
        0xf0 => "CREATE".to_string(),
        0xf1 => "CALL".to_string(),
        0xf2 => "CALLCODE".to_string(),
        0xf3 => "RETURN".to_string(),
        0xf4 => "DELEGATECALL".to_string(),
        0xf5 => "CREATE2".to_string(),
        0xfa => "STATICCALL".to_string(),
        0xfd => "REVERT".to_string(),
        0xfe => "INVALID".to_string(),
        0xff => "SELFDESTRUCT".to_string(),
        op if is_push(op) => format!("PUSH{}", push_size(op)),
        op if is_dup(op) => format!("DUP{}", dup_index(op)),
        op if is_swap(op) => format!("SWAP{}", swap_index(op)),
        _ => format!("UNKNOWN(0x{opcode:02x})"),
    }
}

pub fn static_gas_cost(opcode: u8) -> u64 {
    match opcode {
        0x00 | 0xf3 | 0xfd => 0,
        0x01 | 0x03 | 0x10 | 0x11 | 0x12 | 0x13 | 0x14 | 0x15 | 0x16 | 0x17 | 0x18 | 0x19
        | 0x1a | 0x1b | 0x1c | 0x1d | 0x35 | 0x37 | 0x39 | 0x3e | 0x51 | 0x52 | 0x53 | 0x5e => 3,
        0x02 | 0x04 | 0x05 | 0x06 | 0x07 | 0x0b => 5,
        0x08 | 0x09 => 8,
        0x0a => 10,
        0x20 => 30,
        0x30 | 0x32 | 0x33 | 0x34 | 0x36 | 0x38 | 0x3a | 0x3d | 0x41 | 0x42 | 0x43 | 0x44
        | 0x45 | 0x46 | 0x47 | 0x48 | 0x50 | 0x58 | 0x59 | 0x5a => 2,
        0x31 | 0x3b | 0x3c | 0x3f | 0x54 => 100,
        0x55 => 0,
        0x40 => 20,
        0x56 => 8,
        0x57 => 10,
        0x5b => 1,
        0x60..=0x9f => 3,
        0xa0 => 375,
        0xa1 => 750,
        0xa2 => 1125,
        0xa3 => 1500,
        0xa4 => 1875,
        0xf0 | 0xf5 => 32000,
        0xf1 | 0xf2 | 0xf4 | 0xfa => 700,
        0xff => 5000,
        _ => 0,
    }
}
