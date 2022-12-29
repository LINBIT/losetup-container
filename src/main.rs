use std::env;
use std::error;
use std::ffi;
use std::fs;
use std::io;
use std::mem;
use std::os::raw;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path;
use std::process;

type Error = Box<dyn error::Error + Send + Sync>;

fn main() -> Result<(), Error> {
    let mut args = env::args();
    let _prog_name = args.next().ok_or("Expected program name")?;

    match (args.next(), args.next(), args.next(), args.next()) {
        (Some(a), Some(b), Some(c), None) if a == "-l" && b == "-O" && c == "NAME,BACK-FILE" => (),
        _ => {
            let original_losetup = env::var_os("LOSETUP_CONTAINER_ORIGINAL_LOSETUP")
                .unwrap_or("/usr/sbin/losetup".into());

            if !process::Command::new(original_losetup)
                .args(env::args_os().skip(1))
                .status()?
                .success()
            {
                Err("Failed to run original losetup")?;
            }

            return Ok(());
        }
    }

    let raw_locations = env::var_os("LOSETUP_CONTAINER_BIND_MOUNTS")
        .map(OsStringExt::into_vec)
        .unwrap_or_default();

    let extra_locations: Vec<_> = raw_locations
        .split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
        .map(bytes_to_path)
        .collect();

    println!("NAME       BACK-FILE");
    for entry in fs::read_dir("/sys/block")? {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read DirEntry: {}", e);
                continue;
            }
        };

        let name = entry.file_name();

        if !name.as_bytes().starts_with(b"loop") {
            continue;
        }

        let dev_name = path::Path::new("/dev").join(&name);

        let backing_file = match find_backing_file(&dev_name, &entry.path(), &extra_locations) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Failed to find backing path for '{}': {}",
                    name.to_string_lossy(),
                    e
                );
                continue;
            }
        };

        println!("{} {}", dev_name.display(), backing_file.display());
    }

    Ok(())
}

fn find_backing_file(
    dev_path: &path::Path,
    sys_path: &path::Path,
    extra_locations: &[&path::Path],
) -> Result<path::PathBuf, Error> {
    let mut candidate_locations = Vec::new();

    let (status_candidate, inode) = loop_get_backing_file_and_inode(dev_path)?;
    candidate_locations.push(status_candidate);

    let sys_backing_file = read_sys_backing_file(&sys_path.join("loop/backing_file"))?;
    candidate_locations.push(path::Path::new("/").join(&sys_backing_file));

    for extra in extra_locations {
        let p = extra.join(&sys_backing_file);
        candidate_locations.push(p);
    }

    for candidate in candidate_locations {
        let metadata = match fs::metadata(&candidate) {
            Ok(m) => m,
            Err(_) => {
                continue;
            }
        };

        if metadata.ino() == inode {
            return Ok(candidate);
        }
    }

    // Fall back to the same output as the normal losetup
    return Ok(path::Path::new("/").join(&sys_backing_file));
}

const LO_NAME_SIZE: usize = 64;
const LO_KEY_SIZE: usize = 32;
const LOOP_GET_STATUS64: raw::c_ulong = 0x4C05;

#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; LO_NAME_SIZE],
    lo_crypt_name: [u8; LO_NAME_SIZE],
    lo_encrypt_key: [u8; LO_KEY_SIZE],
    lo_init: [u64; 2],
}

extern "C" {
    pub fn ioctl(fd: raw::c_int, request: raw::c_ulong, ...) -> raw::c_int;
}

fn loop_get_backing_file_and_inode(dev_path: &path::Path) -> Result<(path::PathBuf, u64), Error> {
    let loop_dev = fs::File::open(&dev_path)?;

    let mut info = mem::MaybeUninit::<LoopInfo64>::zeroed();
    let ret = unsafe {
        ioctl(
            loop_dev.as_raw_fd() as raw::c_int,
            LOOP_GET_STATUS64,
            info.as_mut_ptr(),
        )
    };
    if ret < 0 {
        Err(format!(
            "Failed to get status of '{}': {}",
            dev_path.display(),
            io::Error::last_os_error()
        ))?;
    }

    let info = unsafe { info.assume_init() };

    let file_name_end = info
        .lo_file_name
        .iter()
        .position(|b| *b == b'\0')
        .unwrap_or(LO_NAME_SIZE);

    let file_name = bytes_to_path(&info.lo_file_name[..file_name_end]);
    Ok((file_name.into(), info.lo_inode))
}

fn read_sys_backing_file(p: &path::Path) -> Result<path::PathBuf, Error> {
    let raw = fs::read(p)?;
    // Remove leading '/' and trailing '\n'
    let trimmed = &raw[1..raw.len() - 1];
    Ok(ffi::OsStr::from_bytes(trimmed).into())
}

fn bytes_to_path(b: &[u8]) -> &path::Path {
    path::Path::new(ffi::OsStr::from_bytes(b))
}
