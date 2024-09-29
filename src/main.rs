use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// file path. If not specified, read from stdin.
    paths: Vec<PathBuf>,

    /// read batch size.
    #[arg(short, long, default_value_t = 16 << 20)]
    batch_size: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut buf = vec![0u8; args.batch_size];

    for path in &args.paths {
        let mut file = fs::File::open(path)
            .with_context(|| format!("Failed to open file {}", path.display()))?;
        let mut crc32c = 0u32;
        loop {
            // read from source file.
            let n = file
                .read(&mut buf)
                .with_context(|| format!("read source file failed: {}", path.display()))?;
            if n == 0 {
                break;
            }
            crc32c = crc32c::crc32c_append(crc32c, &buf[..n]);
        }
        println!("{:08X} {}", crc32c, path.display());
    }

    if args.paths.is_empty() {
        let mut crc32c = 0;
        let mut line = String::new();
        loop {
            // read from stdin.
            let n = std::io::stdin()
                .read_line(&mut line)
                .with_context(|| "read stdin failed")?;
            if n == 0 {
                break;
            }
            crc32c = crc32c::crc32c_append(crc32c, line.as_bytes());
        }
        println!("{:08X} -", crc32c);
    }

    Ok(())
}
