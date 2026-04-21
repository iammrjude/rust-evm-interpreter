use clap::{Args, Parser, Subcommand};
use evm::evm::{Evm, format_stack};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "evm", about = "A small educational EVM interpreter")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run(RunArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long, conflicts_with = "file", required_unless_present = "file")]
    code: Option<String>,
    #[arg(long, conflicts_with = "code", required_unless_present = "code")]
    file: Option<PathBuf>,
    #[arg(long)]
    trace: bool,
    #[arg(long, default_value_t = 1_000_000)]
    gas: u64,
    #[arg(long, default_value = "0x")]
    calldata: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run(args) => run_command(args),
    }
}

fn run_command(args: RunArgs) -> ExitCode {
    let code = match load_code(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Failed to load code: {err}");
            return ExitCode::from(1);
        }
    };

    let calldata = match parse_hex(&args.calldata) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Invalid calldata: {err}");
            return ExitCode::from(1);
        }
    };

    let mut vm = Evm::new(code)
        .with_gas_limit(args.gas)
        .with_trace(args.trace)
        .with_calldata(calldata);
    let result = vm.run();

    println!("Stack:    {}", format_stack(&result.stack));
    println!("Return:   0x{}", hex::encode(result.return_data));
    println!("Gas used: {}", result.gas_used);
    println!("Status:   {}", status_label(&result.status));

    ExitCode::SUCCESS
}

fn load_code(args: &RunArgs) -> Result<Vec<u8>, String> {
    if let Some(code) = &args.code {
        return parse_hex(code);
    }

    let path = args
        .file
        .as_ref()
        .ok_or_else(|| "missing --file argument".to_string())?;
    let bytes =
        fs::read(path).map_err(|e| format!("unable to read file {}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let stripped: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !stripped.is_empty()
        && stripped
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == 'x' || c == 'X')
    {
        parse_hex(&stripped)
    } else {
        Ok(bytes)
    }
}

fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let trimmed = input.trim();
    let no_prefix = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if no_prefix.is_empty() {
        return Ok(Vec::new());
    }
    let normalized = if no_prefix.len() % 2 == 1 {
        format!("0{no_prefix}")
    } else {
        no_prefix.to_string()
    };
    hex::decode(normalized).map_err(|e| e.to_string())
}

fn status_label(status: &evm::evm::ExitStatus) -> &'static str {
    match status {
        evm::evm::ExitStatus::Stop => "STOP",
        evm::evm::ExitStatus::Return => "RETURN",
        evm::evm::ExitStatus::Revert => "REVERT",
        evm::evm::ExitStatus::SelfDestruct => "SELFDESTRUCT",
        evm::evm::ExitStatus::OutOfGas => "OUT_OF_GAS",
        evm::evm::ExitStatus::StackOverflow => "STACK_OVERFLOW",
        evm::evm::ExitStatus::StackUnderflow => "STACK_UNDERFLOW",
        evm::evm::ExitStatus::BadJumpDestination(_) => "BAD_JUMP_DESTINATION",
        evm::evm::ExitStatus::InvalidOpcode(_) => "INVALID_OPCODE",
        evm::evm::ExitStatus::ReturnDataOutOfBounds => "RETURNDATA_OOB",
        evm::evm::ExitStatus::StaticModeViolation => "STATIC_MODE_VIOLATION",
    }
}
