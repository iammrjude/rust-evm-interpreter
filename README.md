# EVM Assignment (Rust)

This project is a from-scratch educational EVM interpreter written in Rust
and exposed as a CLI binary.

## Important grading note

The assignment sample shows `Gas used: 6` for `0x6001600201`.
Under Yellow Paper static gas costs, the correct value is `9`:

- `PUSH1` = 3
- `PUSH1` = 3
- `ADD` = 3

Total: `9`.

This implementation intentionally follows Yellow Paper and
`ethereum/execution-specs` behavior for gas accounting.

## Dependency rule compliance

- No existing EVM engine/library is used (`revm`, `evm`, `etk`, etc. are not dependencies).
- `Cargo.toml` only uses `primitive-types`, `tiny-keccak`, `clap`, and `hex`.

## What is implemented

### Task 01: Core execution engine

- 256-bit stack (`U256`) with max size 1024
- Byte-addressable expandable memory (zero-padded)
- Persistent storage (`U256 -> U256`)
- Program counter and fetch-decode-execute loop
- Gas accounting with static + dynamic charging
- Pre-scanned `JUMPDEST` set for jump validation

### Task 02: Arithmetic / comparison / bitwise

- Arithmetic: `ADD MUL SUB DIV SDIV MOD SMOD ADDMOD MULMOD EXP SIGNEXTEND`
- Comparison: `LT GT SLT SGT EQ ISZERO`
- Bitwise: `AND OR XOR NOT BYTE SHL SHR SAR`
- Signed behavior uses two's-complement edge-case handling

### Task 03: Stack / memory / storage / calldata / code / return data

- Stack: `PUSH1-32 POP DUP1-16 SWAP1-16`
- Memory: `MLOAD MSTORE MSTORE8 MSIZE MCOPY`
- Storage: `SLOAD SSTORE`
- Calldata: `CALLDATALOAD CALLDATASIZE CALLDATACOPY`
- Code: `CODESIZE CODECOPY EXTCODESIZE EXTCODECOPY EXTCODEHASH`
- Return data: `RETURNDATASIZE RETURNDATACOPY`

### Task 04: Control flow and context

- Control flow: `JUMP JUMPI JUMPDEST PC STOP RETURN REVERT INVALID`
- Transaction context: `ADDRESS BALANCE ORIGIN CALLER CALLVALUE GASPRICE GAS`
- Block context:
  `BLOCKHASH COINBASE TIMESTAMP NUMBER PREVRANDAO GASLIMIT CHAINID SELFBALANCE BASEFEE`

### Task 05: Hashing / logging / system

- Hashing: `SHA3/KECCAK256`
- Logging: `LOG0 LOG1 LOG2 LOG3 LOG4`
- Calls: `CALL CALLCODE DELEGATECALL STATICCALL` with nested frames,
  returndata propagation, gas forwarding cap, and rollback on failure
- Contracts: `CREATE CREATE2 SELFDESTRUCT` with init-code execution and runtime
  deployment

### Task 06: CLI + tests

- `evm run --code 0x...`
- `evm run --file path.bin`
- `evm run --trace`
- 23 integration tests

## Conformance highlights

Implemented spec-aligned behavior for the covered opcodes using the Yellow
Paper + execution-specs and relevant EIPs:

- Dynamic memory expansion gas: `C_mem(a) = 3a + floor(a^2 / 512)`
- Dynamic copy gas for copy-family opcodes and `MCOPY`
- Dynamic `EXP` gas by exponent byte length
- Dynamic `SHA3` and `LOG*` data gas
- `CALL*` gas forwarding cap (`all but one 64th`, EIP-150 style)
- `CREATE2` hash-word gas
- `RETURN/REVERT` memory expansion charging
- Exceptional halts consume remaining gas (`InvalidOpcode`, bad jump, OOG paths)
- `CREATE` address derivation from RLP(`sender`, `nonce`)
- EIP-2929 warm/cold access-list charging for account and storage accesses
- EIP-3529 refund accounting (including refund cap at 1/5 of gas used)

## Project structure

```text
evm/
|-- Cargo.toml
|-- README.md
|-- examples/
|   `-- add.bin
|-- src/
|   |-- main.rs
|   |-- lib.rs
|   |-- evm.rs
|   |-- opcodes.rs
|   |-- stack.rs
|   |-- memory.rs
|   `-- storage.rs
`-- tests/
    `-- integration.rs
```

## Build

```bash
cargo build --release
```

Binary path:

```bash
./target/release/evm
```

## Run

Run from hex:

```bash
cargo run -- run --code 0x600160020100
```

or with built binary:

```bash
./target/release/evm run --code 0x600160020100
```

Run from file:

```bash
cargo run -- run --file examples/add.bin
```

Trace mode:

```bash
cargo run -- run --code 0x600160020100 --trace
```

Example output:

```text
Stack:    [0x3]
Return:   0x
Gas used: 9
Status:   STOP
```

> For `0x600160020100`, gas used is `9`
> (`PUSH1=3`, `PUSH1=3`, `ADD=3`, `STOP=0`) under Yellow Paper static gas.
>
> `0x6001600201` (without trailing `STOP`) also uses `9` gas.
> The sample that shows `6` is not Yellow-Paper gas accounting.

## CLI command combinations

Rules:

- Use exactly one input source: `--code` or `--file`.
- Optional modifiers are `--trace`, `--gas`, and `--calldata`.

All valid `run` command shapes:

```bash
evm run --code <BYTECODE_HEX>
evm run --code <BYTECODE_HEX> --trace
evm run --code <BYTECODE_HEX> --gas <N>
evm run --code <BYTECODE_HEX> --calldata <CALLDATA_HEX>
evm run --code <BYTECODE_HEX> --trace --gas <N>
evm run --code <BYTECODE_HEX> --trace --calldata <CALLDATA_HEX>
evm run --code <BYTECODE_HEX> --gas <N> --calldata <CALLDATA_HEX>
evm run --code <BYTECODE_HEX> --trace --gas <N> --calldata <CALLDATA_HEX>

evm run --file <PATH_TO_BIN>
evm run --file <PATH_TO_BIN> --trace
evm run --file <PATH_TO_BIN> --gas <N>
evm run --file <PATH_TO_BIN> --calldata <CALLDATA_HEX>
evm run --file <PATH_TO_BIN> --trace --gas <N>
evm run --file <PATH_TO_BIN> --trace --calldata <CALLDATA_HEX>
evm run --file <PATH_TO_BIN> --gas <N> --calldata <CALLDATA_HEX>
evm run --file <PATH_TO_BIN> --trace --gas <N> --calldata <CALLDATA_HEX>
```

## Why `evm run --code ...` may fail on your machine

If `evm` is not on your `PATH`, the shell cannot find that command.
That is why `cargo run -- run --code ...` works reliably in this project.

You can run it in any of these ways:

```bash
cargo run -- run --code 0x6001600201
./target/debug/evm run --code 0x6001600201
./target/release/evm run --code 0x6001600201
```

On Windows PowerShell, use:

```powershell
.\target\debug\evm.exe run --code 0x6001600201
.\target\release\evm.exe run --code 0x6001600201
```

Optional install for global command access:

```bash
cargo install --path .
```

What this does:

- Builds this local project and installs its binary globally as `evm`.
- Installs to Cargo's bin directory:
  - Linux/macOS: `~/.cargo/bin`
  - Windows: `%USERPROFILE%\\.cargo\\bin`
- If needed, add that directory to your `PATH`.
- Reinstall after local code changes with:

```bash
cargo install --path . --force
```

After installation, `evm run --code ...` should work from anywhere.

## Architecture notes

- `stack.rs`: bounded stack + DUP/SWAP helpers
- `memory.rs`: expandable byte memory + typed word operations
- `storage.rs`: persistent key-value store
- `opcodes.rs`: opcode names/helpers + static gas base table
- `evm.rs`: VM state machine, opcode dispatch, gas logic, nested execution
- `main.rs`: CLI parser + code loading + output formatting

## Test suite

Run:

```bash
cargo test
```

Current output:

```text
running 23 tests
test calldatacopy_charges_memory_and_copy_gas ... ok
test calldata_copy_and_mload_work ... ok
test call_executes_external_contract_code ... ok
test call_revert_sets_returndata ... ok
test balance_uses_cold_then_warm_access_cost ... ok
test create_deploys_runtime_code ... ok
test delegatecall_uses_caller_storage_context ... ok
test exp_charges_dynamic_gas_by_exponent_size ... ok
test gas_exhaustion_is_reported ... ok
test integer_overflow_wraps ... ok
test invalid_jump_destination_fails ... ok
test invalid_opcode_consumes_all_gas ... ok
test jumpi_to_valid_jumpdest ... ok
test multi_opcode_program_executes ... ok
test revert_returns_data ... ok
test returndatacopy_out_of_bounds_fails ... ok
test sar_sign_extends ... ok
test sdiv_handles_negative_values ... ok
test simple_add_program ... ok
test sload_uses_cold_then_warm_slot_cost ... ok
test sstore_dynamic_gas_is_enforced ... ok
test sstore_refund_is_capped_at_one_fifth ... ok
test stack_overflow_is_reported ... ok

test result: ok. 23 passed; 0 failed
```
