use std::{env, error::Error, fs::File, os::unix::fs::FileExt, path::Path};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use linkedin_mcp::{
    config::AppConfig,
    linkedin::{token_store::TokenStore, TokenSet},
};
use ring::digest::{digest, SHA256};
use rustix::{
    fs::{self, FileType, FlockOperation, Mode, OFlags},
    process::geteuid,
    time::{clock_gettime, ClockId},
};

const RUNTIME_ERROR: &str =
    "could not use the auth coordination file; check the private XDG_RUNTIME_DIR for this Linux user";
const TIMESTAMP_BYTES: u64 = size_of::<i128>() as u64;
const CANCELLED: &str = "local logout cancelled this LinkedIn login; start authorization again";

/// Linux's monotonic clock has the same origin in both CLI processes.
pub(super) fn monotonic_now() -> i128 {
    let now = clock_gettime(ClockId::Monotonic);
    i128::from(now.tv_sec) * 1_000_000_000 + i128::from(now.tv_nsec)
}

/// Coordinates CLI credential changes across processes using a file that holds
/// only logout timestamps. The file stays unlocked during browser consent.
pub(super) struct AuthSession(File);

impl AuthSession {
    pub(super) fn open(config: &AppConfig) -> Result<Self, &'static str> {
        let client_id = config
            .linkedin_client_id
            .as_deref()
            .ok_or("set LINKEDIN_CLIENT_ID to select the LinkedIn credential")?;
        let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or(RUNTIME_ERROR)?;
        let runtime = Path::new(&runtime);
        if !runtime.is_absolute() {
            return Err(RUNTIME_ERROR);
        }

        let dir = fs::open(
            runtime,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| RUNTIME_ERROR)?;
        let info = fs::fstat(&dir).map_err(|_| RUNTIME_ERROR)?;
        if info.st_uid != geteuid().as_raw() || info.st_mode & 0o777 != 0o700 {
            return Err(RUNTIME_ERROR);
        }

        let app = URL_SAFE_NO_PAD.encode(digest(&SHA256, client_id.as_bytes()).as_ref());
        let file_name = format!("linkedin-mcp-auth-{app}");
        let file = fs::openat(
            &dir,
            file_name.as_str(),
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| RUNTIME_ERROR)?;
        Ok(Self(file.into()))
    }

    fn lock(self) -> Result<(File, u64), &'static str> {
        let file = self.0;
        fs::flock(&file, FlockOperation::LockExclusive).map_err(|_| RUNTIME_ERROR)?;
        let info = fs::fstat(&file).map_err(|_| RUNTIME_ERROR)?;
        if FileType::from_raw_mode(info.st_mode) != FileType::RegularFile
            || info.st_uid != geteuid().as_raw()
            || info.st_mode & 0o077 != 0
            || info.st_nlink != 1
        {
            return Err(RUNTIME_ERROR);
        }
        let len = u64::try_from(info.st_size).map_err(|_| RUNTIME_ERROR)?;
        Ok((file, len))
    }

    pub(super) fn save(
        self,
        store: &TokenStore,
        tokens: &TokenSet,
        started_at: i128,
    ) -> Result<(), Box<dyn Error>> {
        let (file, len) = self.lock()?;
        if len % TIMESTAMP_BYTES != 0 {
            return Err(RUNTIME_ERROR.into());
        }
        if len != 0 {
            let mut timestamp = [0; size_of::<i128>()];
            file.read_exact_at(&mut timestamp, len - TIMESTAMP_BYTES)
                .map_err(|_| RUNTIME_ERROR)?;
            if started_at <= i128::from_le_bytes(timestamp) {
                return Err(CANCELLED.into());
            }
        }
        store.save(tokens)?;
        Ok(())
    }

    pub(super) fn logout(self, store: &TokenStore) -> Result<(), Box<dyn Error>> {
        let (file, len) = self.lock()?;
        // Replace any partial tail without overwriting a complete marker.
        let offset = len - len % TIMESTAMP_BYTES;
        // Invalidate existing attempts even if the process exits during deletion.
        record_logout_time(&file, offset)?;
        store.delete()?;

        // Also invalidate attempts that started while deletion held the lock.
        record_logout_time(&file, offset + TIMESTAMP_BYTES)?;
        Ok(())
    }
}

fn record_logout_time(file: &File, offset: u64) -> Result<(), &'static str> {
    file.write_all_at(&monotonic_now().to_le_bytes(), offset)
        .map_err(|_| RUNTIME_ERROR)?;
    file.sync_all().map_err(|_| RUNTIME_ERROR)
}
