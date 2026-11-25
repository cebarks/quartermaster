//! Directory-attached generation labels, separate from native mask inode identity.
//! Missing metadata is never repaired once a journal has granted ownership.
use super::*;
use std::os::fd::AsRawFd;

const NAME: &std::ffi::CStr = c"user.quma.overlay-generation.v1";
const LENGTH: usize = 67; // v1: followed by 256 random bits in lowercase hex

pub(super) fn valid(token: &str) -> bool {
    token.len() == LENGTH
        && token.starts_with("v1:")
        && token.as_bytes()[3..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub(super) fn read(scope: &Scope) -> Result<Option<String>> {
    super::super::private_metadata(&scope.dir.dir_metadata()?.into(), false)?;
    // cap-std may hold an O_PATH descriptor; xattr syscalls require a real fd.
    let directory = scope.dir.open(".")?;
    let mut value = [0u8; LENGTH + 1];
    // A fixed buffer bounds hostile/unsupported metadata; ERANGE refuses too.
    let size = unsafe {
        libc::fgetxattr(
            directory.as_raw_fd(),
            NAME.as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
        )
    };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENODATA) {
            return Ok(None);
        }
        return Err(error).context("cannot read durable directory generation proof");
    }
    let token = std::str::from_utf8(&value[..size as usize])?;
    if !valid(token) {
        bail!("malformed directory generation proof");
    }
    Ok(Some(token.to_owned()))
}

pub(super) fn initialize(scope: &Scope) -> Result<String> {
    let token = if let Some(token) = read(scope)? {
        token
    } else {
        // Fresh cryptographic-strength labels are not derived from recyclable
        // inode numbers, timestamps, client indices or process-local counters.
        let mut random = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let mut token = String::from("v1:");
        use std::fmt::Write;
        for byte in random {
            write!(&mut token, "{byte:02x}")?;
        }
        let directory = scope.dir.open(".")?;
        if unsafe {
            libc::fsetxattr(
                directory.as_raw_fd(),
                NAME.as_ptr(),
                token.as_ptr().cast(),
                token.len(),
                libc::XATTR_CREATE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error())
                .context("cannot create durable directory generation proof");
        }
        token
    };
    // Also sync an existing token: a prior unbound initialization may have
    // stopped after create but before fsync. No authority existed at that point.
    scope.sync()?;
    Ok(token)
}

pub(super) fn initialize_generation(dirs: &QumaDirs, client: u32) -> Result<Generation> {
    let overlay = Scope::open(&dirs.headless_overlay(client))?;
    let upper = Scope::open(&dirs.headless_upper(client))?;
    initialize(&overlay)?;
    initialize(&upper)?;
    overlay.sync()?;
    // Persist every ancestor name, including freshly created backing trees.
    let overlay_path = dirs.headless_overlay(client);
    let relative = overlay_path.strip_prefix(&dirs.root)?;
    let root = Scope::open(&dirs.root)?;
    for parent in relative
        .ancestors()
        .skip(1)
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        root.dir.open_dir(parent)?.open(".")?.sync_all()?;
    }
    root.sync()?;
    generation(dirs, client)
}

pub(super) fn verify_retiring(dirs: &QumaDirs, client: u32, old: &Generation) -> Result<()> {
    let root = Scope::open(&dirs.root)?;
    let overlay = dirs.headless_overlay(client);
    if root.identity(overlay.strip_prefix(&dirs.root)?)?.as_ref() != Some(&old.overlay)
        || read(&Scope::open(&overlay)?)? != old.overlay_proof
    {
        bail!("retiring overlay generation proof changed");
    }
    let upper = dirs.headless_upper(client);
    if let Some(id) = root.identity(upper.strip_prefix(&dirs.root)?)? {
        if id != old.upper || read(&Scope::open(&upper)?)? != old.upper_proof {
            bail!("retiring upper generation proof changed");
        }
    }
    Ok(())
}
