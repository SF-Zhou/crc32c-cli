use aligned_utils::bytes::AlignedBytes;
use anyhow::{Context, Result};
use clap::Parser;
use scoped_threadpool::Pool;
use std::cell::RefCell;
use std::fs::{self, File};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// file path. If not specified, read from stdin.
    paths: Vec<PathBuf>,

    /// number of threads.
    #[arg(short, long, default_value_t = 1)]
    threads: u32,

    /// fill zero when the read length is less than expected.
    #[arg(long, default_value_t = false)]
    fill_zero: bool,
}

const BLOCK_SIZE: usize = 16 << 20; // 16MiB.
const ALIGN_SIZE: usize = 512;
thread_local! {
    static TLS: RefCell<AlignedBytes> = RefCell::new(AlignedBytes::new_zeroed(BLOCK_SIZE, ALIGN_SIZE));
}

fn parallel_read(file: &File, path: &Path, pool: &mut Pool, fill_zero: bool) -> Result<u32> {
    let mut start = 0u64;
    let mut crc32c = 0u32;
    let file_size = file
        .metadata()
        .with_context(|| format!("get file metadata failed: {}", path.display()))?
        .size();
    loop {
        let mut vec: Vec<Result<(u64, u32, bool)>> = vec![];
        vec.resize_with(pool.thread_count() as usize, || Ok((0, 0, true)));

        pool.scoped(|scoped| {
            for (i, r) in vec.iter_mut().enumerate() {
                scoped.execute(move || {
                    let offset = start + (i * BLOCK_SIZE) as u64;
                    *r = TLS.with(|v| -> Result<(u64, u32, bool)> {
                        let mut buf = v.borrow_mut();
                        let mut n = file.read_at(&mut buf, offset).with_context(|| {
                            format!("read source file failed: {}", path.display())
                        })?;
                        if fill_zero && offset < file_size {
                            let expect =
                                std::cmp::min(file_size - offset, BLOCK_SIZE as u64) as usize;
                            if n < expect {
                                buf[n..expect].fill(0);
                                n = expect;
                            }
                        }
                        let crc32c = crc32c::crc32c(&buf[..n]);
                        Ok((n as u64, crc32c, n != BLOCK_SIZE))
                    });
                });
            }
        });

        let (len, crc, finished) =
            vec.into_iter()
                .try_fold((0u64, 0u32, false), |a, b| -> Result<(u64, u32, bool)> {
                    let (len_a, crc_a, finished) = a;
                    if finished {
                        return Ok(a);
                    }
                    match b {
                        Ok((len_b, crc_b, finished_b)) => {
                            let crc = crc32c::crc32c_combine(crc_a, crc_b, len_b as usize);
                            Ok((len_a + len_b, crc, finished_b))
                        }
                        Err(_) => b,
                    }
                })?;
        crc32c = crc32c::crc32c_combine(crc32c, crc, len as _);
        start += len;
        if finished {
            return Ok(crc32c);
        }
    }
}

#[cfg(target_os = "linux")]
fn open(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
}
#[cfg(not(target_os = "linux"))]
fn open(path: &Path) -> std::io::Result<fs::File> {
    fs::File::open(path)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut pool = Pool::new(args.threads);

    for path in &args.paths {
        let file = open(path).with_context(|| format!("Failed to open file {}", path.display()))?;
        let crc32c = parallel_read(&file, path, &mut pool, args.fill_zero)?;
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
