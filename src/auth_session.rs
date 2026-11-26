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
const STATE_BYTES: u64 = 32;
const STATE_MAGIC: &[u8; 4] = b"LIAS";
const STATE_VERSION: u8 = 1;
const COMPLETED: u8 = 0;
const PENDING: u8 = 1;
const CANCELLED: &str = "local logout cancelled this LinkedIn login; start authorization again";
const LOGOUT_PENDING: &str =
    "a previous LinkedIn logout was interrupted; run auth logout again to complete it";

/// Linux's monotonic clock has the same origin in both CLI processes.
pub(super) fn monotonic_now() -> i128 {
    let now = clock_gettime(ClockId::Monotonic);
    i128::from(now.tv_sec) * 1_000_000_000 + i128::from(now.tv_nsec)
}

/// The state file stays unlocked during browser consent.
pub(super) struct AuthSession {
    state: File,
    dir: File,
    legacy_name: String,
}

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
        let dir = File::from(dir);
        Ok(Self {
            state: open_file(&dir, &format!("linkedin-mcp-auth-v2-{app}"))?,
            dir,
            legacy_name: format!("linkedin-mcp-auth-{app}"),
        })
    }

    pub(super) fn save(
        self,
        store: &TokenStore,
        tokens: &TokenSet,
        started_at: i128,
    ) -> Result<(), Box<dyn Error>> {
        let len = lock(&self.state)?;
        let state = read_state(&self.state, len)?;
        match state {
            Some(StateRecord {
                status: PENDING, ..
            }) => return Err(LOGOUT_PENDING.into()),
            Some(StateRecord {
                status: COMPLETED,
                timestamp,
            }) if started_at <= timestamp => return Err(CANCELLED.into()),
            Some(_) | None => {}
        }
        store.save(tokens)?;
        Ok(())
    }

    pub(super) fn logout(self, store: &TokenStore) -> Result<(), Box<dyn Error>> {
        // All two-file operations lock v2 first, then legacy, through completion.
        lock(&self.state)?;
        let legacy = open_file(&self.dir, &self.legacy_name)?;
        lock(&legacy)?;
        write_state(
            &self.state,
            StateRecord {
                status: PENDING,
                timestamp: monotonic_now(),
            },
        )?;
        // Change the length in place so even a login holding an older open
        // descriptor sees the fence. Both legacy formats reject one byte.
        legacy.set_len(1).map_err(|_| RUNTIME_ERROR)?;
        legacy.sync_all().map_err(|_| RUNTIME_ERROR)?;
        // Either name may have been created by this logout.
        self.dir.sync_all().map_err(|_| RUNTIME_ERROR)?;
        store.delete()?;

        // Successful completion permits new logins and invalidates older ones.
        write_state(
            &self.state,
            StateRecord {
                status: COMPLETED,
                timestamp: monotonic_now(),
            },
        )?;
        Ok(())
    }
}

fn open_file(dir: &File, name: &str) -> Result<File, &'static str> {
    let file = fs::openat(
        dir,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| RUNTIME_ERROR)?;
    Ok(file.into())
}

fn lock(file: &File) -> Result<u64, &'static str> {
    fs::flock(file, FlockOperation::LockExclusive).map_err(|_| RUNTIME_ERROR)?;
    let info = fs::fstat(file).map_err(|_| RUNTIME_ERROR)?;
    if FileType::from_raw_mode(info.st_mode) != FileType::RegularFile
        || info.st_uid != geteuid().as_raw()
        || info.st_mode & 0o077 != 0
        || info.st_nlink != 1
    {
        return Err(RUNTIME_ERROR);
    }
    u64::try_from(info.st_size).map_err(|_| RUNTIME_ERROR)
}

#[derive(Clone, Copy)]
struct StateRecord {
    status: u8,
    timestamp: i128,
}

fn encode_state(state: StateRecord) -> [u8; STATE_BYTES as usize] {
    let mut bytes = [0; STATE_BYTES as usize];
    bytes[..4].copy_from_slice(STATE_MAGIC);
    bytes[4] = STATE_VERSION;
    bytes[5] = state.status;
    bytes[8..24].copy_from_slice(&state.timestamp.to_le_bytes());
    let checksum = digest(&SHA256, &bytes[..24]);
    bytes[24..].copy_from_slice(&checksum.as_ref()[..8]);
    bytes
}

fn decode_state(bytes: &[u8; STATE_BYTES as usize]) -> Result<StateRecord, &'static str> {
    let checksum = digest(&SHA256, &bytes[..24]);
    if &bytes[..4] != STATE_MAGIC
        || bytes[4] != STATE_VERSION
        || !matches!(bytes[5], COMPLETED | PENDING)
        || bytes[6..8] != [0; 2]
        || bytes[24..] != checksum.as_ref()[..8]
    {
        return Err(RUNTIME_ERROR);
    }
    let timestamp = i128::from_le_bytes(bytes[8..24].try_into().map_err(|_| RUNTIME_ERROR)?);
    Ok(StateRecord {
        status: bytes[5],
        timestamp,
    })
}

fn read_state(file: &File, len: u64) -> Result<Option<StateRecord>, &'static str> {
    if len == 0 {
        return Ok(None);
    }
    if len != STATE_BYTES {
        return Err(RUNTIME_ERROR);
    }
    let mut bytes = [0; STATE_BYTES as usize];
    file.read_exact_at(&mut bytes, 0)
        .map_err(|_| RUNTIME_ERROR)?;
    decode_state(&bytes).map(Some)
}

fn write_state(file: &File, state: StateRecord) -> Result<(), &'static str> {
    file.write_all_at(&encode_state(state), 0)
        .map_err(|_| RUNTIME_ERROR)?;
    file.set_len(STATE_BYTES).map_err(|_| RUNTIME_ERROR)?;
    file.sync_all().map_err(|_| RUNTIME_ERROR)
}
