use std::io::{self, Read};

use declmig_runner::{admit, FencedExecution, MAX_INPUT_BYTES};

fn main() {
    if let Err(error) = run() {
        eprintln!("declmig-runner admission failed: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Repository seed transport: bounded stdin only. Production transport will be
    // a private queue/worker adapter; this binary intentionally opens no socket.
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err("execution envelope exceeds 64 KiB".into());
    }

    let request: FencedExecution = serde_json::from_slice(&bytes)?;
    let receipt = admit(request)?;
    serde_json::to_writer(io::stdout(), &receipt)?;
    Ok(())
}
