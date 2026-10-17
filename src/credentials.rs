//! API keys stay in the current user's OS credential store, never configuration.
//! These blocking operations belong on a worker thread: the OS may request unlock.

const SERVICE: &str = "com.screencue.desktop";
const MAX_SECRET: usize = 2560;
const STORE_ERROR: &str = "The system credential store could not save this API key.";
const READ_ERROR: &str = "The system credential store could not read this API key.";
const DELETE_ERROR: &str = "The system credential store could not remove this API key.";

fn validate_account(account: &str) -> Result<(), String> {
    if account.is_empty()
        || account.len() > 128
        || !account
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-:".contains(&c))
    {
        return Err("Credential account must contain 1 to 128 ASCII letters, digits, dots, underscores, hyphens or colons.".into());
    }
    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), String> {
    if secret.is_empty()
        || secret.len() > MAX_SECRET
        || secret.trim() != secret
        || secret.chars().any(char::is_control)
    {
        return Err("API key must contain 1 to 2560 UTF-8 bytes, without control characters or surrounding whitespace.".into());
    }
    Ok(())
}

fn decode_secret(bytes: &[u8]) -> Result<String, String> {
    let secret = std::str::from_utf8(bytes).map_err(|_| READ_ERROR.to_string())?;
    validate_secret(secret).map_err(|_| READ_ERROR.to_string())?;
    Ok(secret.to_owned())
}

pub fn set(account: &str, secret: &str) -> Result<(), String> {
    validate_account(account)?;
    validate_secret(secret)?;
    backend::set(account, secret)
}

pub fn get(account: &str) -> Result<Option<String>, String> {
    validate_account(account)?;
    backend::get(account)
}

pub fn delete(account: &str) -> Result<(), String> {
    validate_account(account)?;
    backend::delete(account)
}

/// Report whether this account is stored without returning its key to the caller.
/// macOS requests metadata only; Linux lookup may still require keyring unlock.
pub fn exists(account: &str) -> Result<bool, String> {
    validate_account(account)?;
    backend::exists(account)
}

pub fn backend_name() -> &'static str {
    backend::NAME
}

#[cfg(target_os = "windows")]
mod backend {
    use super::*;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::*;

    pub const NAME: &str = "Windows Credential Manager";

    fn target(account: &str) -> Vec<u16> {
        format!("{SERVICE}:api:{account}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let mut target = target(account);
        let mut username: Vec<u16> = account.encode_utf16().chain(Some(0)).collect();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            Err(STORE_ERROR.into())
        } else {
            Ok(())
        }
    }

    pub fn get(account: &str) -> Result<Option<String>, String> {
        let target = target(account);
        let mut credential = std::ptr::null_mut();
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(READ_ERROR.into())
            };
        }
        struct FreeCredential(*mut CREDENTIALW);
        impl Drop for FreeCredential {
            fn drop(&mut self) {
                unsafe { CredFree(self.0.cast()) }
            }
        }
        if credential.is_null() {
            return Err(READ_ERROR.into());
        }
        let owned = FreeCredential(credential);
        let data = unsafe { &*owned.0 };
        if data.CredentialBlobSize == 0
            || data.CredentialBlobSize as usize > MAX_SECRET
            || data.CredentialBlob.is_null()
        {
            return Err(READ_ERROR.into());
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(data.CredentialBlob, data.CredentialBlobSize as usize)
        };
        decode_secret(bytes).map(Some)
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let target = target(account);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0
            || unsafe { GetLastError() } == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err(DELETE_ERROR.into())
        }
    }

    pub fn exists(account: &str) -> Result<bool, String> {
        get(account).map(|secret| secret.is_some())
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    pub const NAME: &str = "macOS Keychain";
    type Object = *const c_void;
    const NOT_FOUND: i32 = -25300;
    const DUPLICATE: i32 = -25299;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        static kSecClass: Object;
        static kSecClassGenericPassword: Object;
        static kSecAttrService: Object;
        static kSecAttrAccount: Object;
        static kSecValueData: Object;
        static kSecReturnData: Object;
        static kSecReturnAttributes: Object;
        static kSecMatchLimit: Object;
        static kSecMatchLimitOne: Object;
        fn SecItemAdd(attributes: Object, result: *mut Object) -> i32;
        fn SecItemUpdate(query: Object, attributes: Object) -> i32;
        fn SecItemCopyMatching(query: Object, result: *mut Object) -> i32;
        fn SecItemDelete(query: Object) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        static kCFBooleanTrue: Object;
        fn CFDictionaryCreateMutable(
            allocator: Object,
            capacity: isize,
            keys: *const c_void,
            values: *const c_void,
        ) -> Object;
        fn CFDictionarySetValue(dictionary: Object, key: Object, value: Object);
        fn CFStringCreateWithBytes(
            allocator: Object,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            external: u8,
        ) -> Object;
        fn CFDataCreate(allocator: Object, bytes: *const u8, length: isize) -> Object;
        fn CFDataGetLength(data: Object) -> isize;
        fn CFDataGetBytePtr(data: Object) -> *const u8;
        fn CFDataGetTypeID() -> usize;
        fn CFGetTypeID(object: Object) -> usize;
        fn CFRelease(object: Object);
    }

    struct Owned(Object);
    impl Owned {
        fn checked(value: Object) -> Result<Self, String> {
            if value.is_null() {
                Err("The system credential store could not allocate a request.".into())
            } else {
                Ok(Self(value))
            }
        }
        fn dictionary() -> Result<Self, String> {
            unsafe {
                Self::checked(CFDictionaryCreateMutable(
                    null(),
                    0,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                ))
            }
        }
        fn string(value: &str) -> Result<Self, String> {
            unsafe {
                Self::checked(CFStringCreateWithBytes(
                    null(),
                    value.as_ptr(),
                    value.len() as isize,
                    0x08000100,
                    0,
                ))
            }
        }
        fn insert(&self, key: Object, value: Object) {
            unsafe { CFDictionarySetValue(self.0, key, value) }
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    fn query(account: &str) -> Result<Owned, String> {
        let query = Owned::dictionary()?;
        let service = Owned::string(SERVICE)?;
        let account = Owned::string(account)?;
        unsafe {
            query.insert(kSecClass, kSecClassGenericPassword);
            query.insert(kSecAttrService, service.0);
            query.insert(kSecAttrAccount, account.0);
        }
        Ok(query)
    }

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let query = query(account)?;
        let attributes = Owned::dictionary()?;
        let data = unsafe {
            Owned::checked(CFDataCreate(null(), secret.as_ptr(), secret.len() as isize))?
        };
        unsafe {
            attributes.insert(kSecValueData, data.0);
            let mut status = SecItemUpdate(query.0, attributes.0);
            if status == NOT_FOUND {
                query.insert(kSecValueData, data.0);
                status = SecItemAdd(query.0, null_mut());
                if status == DUPLICATE {
                    // Another app instance may have created this exact account.
                    let fresh = self::query(account)?;
                    status = SecItemUpdate(fresh.0, attributes.0);
                }
            }
            if status == 0 {
                Ok(())
            } else {
                Err(STORE_ERROR.into())
            }
        }
    }

    pub fn get(account: &str) -> Result<Option<String>, String> {
        let query = query(account)?;
        unsafe {
            query.insert(kSecReturnData, kCFBooleanTrue);
            query.insert(kSecMatchLimit, kSecMatchLimitOne);
            let mut value = null();
            let status = SecItemCopyMatching(query.0, &mut value);
            if status == NOT_FOUND {
                return Ok(None);
            }
            if status != 0 {
                return Err(READ_ERROR.into());
            }
            let data = Owned::checked(value)?;
            if CFGetTypeID(data.0) != CFDataGetTypeID() {
                return Err(READ_ERROR.into());
            }
            let length = CFDataGetLength(data.0);
            let bytes = CFDataGetBytePtr(data.0);
            if length <= 0 || length as usize > MAX_SECRET || bytes.is_null() {
                return Err(READ_ERROR.into());
            }
            decode_secret(std::slice::from_raw_parts(bytes, length as usize)).map(Some)
        }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let query = query(account)?;
        match unsafe { SecItemDelete(query.0) } {
            0 | NOT_FOUND => Ok(()),
            _ => Err(DELETE_ERROR.into()),
        }
    }

    pub fn exists(account: &str) -> Result<bool, String> {
        let query = query(account)?;
        unsafe {
            query.insert(kSecReturnAttributes, kCFBooleanTrue);
            query.insert(kSecMatchLimit, kSecMatchLimitOne);
            let mut result = null();
            let status = SecItemCopyMatching(query.0, &mut result);
            if !result.is_null() {
                drop(Owned(result));
            }
            match status {
                0 => Ok(true),
                NOT_FOUND => Ok(false),
                _ => Err(READ_ERROR.into()),
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::*;
    use std::ffi::{c_int, c_ulong};
    use std::io::{ErrorKind, Read, Write};
    use std::net::Shutdown;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::time::{Duration, Instant};

    pub const NAME: &str = "Linux Secret Service (libsecret)";
    const TIMEOUT: Duration = Duration::from_secs(10);
    const UNAVAILABLE: &str = "Secret Service is unavailable. Install libsecret's secret-tool and enable a desktop keyring.";

    const PR_SET_PDEATHSIG: c_int = 1;
    const SIGKILL: c_int = 9;
    extern "C" {
        fn prctl(option: c_int, ...) -> c_int;
        fn getppid() -> c_int;
    }

    fn protect_parent_exit(command: &mut Command, parent_pid: u32) {
        // A worker can be terminated when the UI process exits before its normal
        // timeout/drop cleanup runs. Ask the kernel to kill this direct child.
        // Only syscalls and allocation-free errors are used after fork.
        unsafe {
            command.pre_exec(move || {
                if prctl(
                    PR_SET_PDEATHSIG,
                    SIGKILL as c_ulong,
                    0 as c_ulong,
                    0 as c_ulong,
                    0 as c_ulong,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have exited between fork and prctl. In that
                // race no signal is delivered, so abort before executing a tool.
                if getppid() as u32 != parent_pid {
                    return Err(std::io::Error::from_raw_os_error(4)); // EINTR
                }
                Ok(())
            });
        }
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            // Every return path reaps the direct child, including timeout/errors.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct Output {
        status: ExitStatus,
        bytes: Vec<u8>,
        had_error: bool,
    }

    fn channel() -> Result<(UnixStream, Stdio), String> {
        let (parent, child) = UnixStream::pair().map_err(|_| UNAVAILABLE.to_string())?;
        parent
            .set_nonblocking(true)
            .map_err(|_| UNAVAILABLE.to_string())?;
        Ok((parent, Stdio::from(OwnedFd::from(child))))
    }

    fn drain(socket: &mut UnixStream, output: &mut Vec<u8>, discard: bool) -> Result<bool, String> {
        // Yield to the process deadline even if a broken helper floods stderr.
        for _ in 0..32 {
            let mut bytes = [0u8; 512];
            match socket.read(&mut bytes) {
                Ok(0) => return Ok(true),
                Ok(length) => {
                    if discard {
                        // A marker distinguishes a missing item from a failed service;
                        // never retain or expose secret-tool diagnostics.
                        if output.is_empty() {
                            output.push(1);
                        }
                    } else {
                        if output.len() + length > MAX_SECRET {
                            return Err(READ_ERROR.into());
                        }
                        output.extend_from_slice(&bytes[..length]);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(READ_ERROR.into()),
            }
        }
        Ok(false)
    }

    fn run(
        executable: &str,
        operation: &str,
        account: &str,
        secret: &str,
        timeout: Duration,
    ) -> Result<Output, String> {
        let (mut input, child_input) = channel()?;
        let (mut output, child_output) = channel()?;
        let (mut error, child_error) = channel()?;
        let mut command = Command::new(executable);
        command.arg(operation);
        if operation == "store" {
            command.args(["--label=ScreenCue API key"]);
        }
        command.args(["service", SERVICE, "account", account]);
        command
            .stdin(child_input)
            .stdout(child_output)
            .stderr(child_error);
        protect_parent_exit(&mut command, std::process::id());
        let child = command.spawn().map_err(|_| UNAVAILABLE.to_string())?;
        // Drop the parent's copies of the child-side sockets so EOF is observable.
        drop(command);
        let mut child = Process(child);
        let deadline = Instant::now() + timeout;
        let mut sent = 0;
        let mut input_closed = false;
        let mut bytes = Vec::new();
        let mut diagnostic = Vec::new();
        let mut status = None;
        loop {
            if Instant::now() >= deadline {
                return Err(
                    "The system credential store timed out. Unlock the desktop keyring and retry."
                        .into(),
                );
            }
            if sent < secret.len() {
                match input.write(&secret.as_bytes()[sent..]) {
                    Ok(0) => return Err(STORE_ERROR.into()),
                    Ok(length) => sent += length,
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return Err(STORE_ERROR.into()),
                }
            }
            if sent == secret.len() && !input_closed {
                input
                    .shutdown(Shutdown::Write)
                    .map_err(|_| STORE_ERROR.to_string())?;
                input_closed = true;
            }
            let output_closed = drain(&mut output, &mut bytes, false)?;
            let error_closed = drain(&mut error, &mut diagnostic, true)?;
            if status.is_none() {
                status = child.0.try_wait().map_err(|_| UNAVAILABLE.to_string())?;
            }
            if let Some(status) = status {
                if output_closed && error_closed {
                    return Ok(Output {
                        status,
                        bytes,
                        had_error: !diagnostic.is_empty(),
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let output = run("secret-tool", "store", account, secret, TIMEOUT)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(STORE_ERROR.into())
        }
    }

    pub fn get(account: &str) -> Result<Option<String>, String> {
        let output = run("secret-tool", "lookup", account, "", TIMEOUT)?;
        if output.status.success() {
            decode_secret(&output.bytes).map(Some)
        } else if output.status.code() == Some(1) && !output.had_error && output.bytes.is_empty() {
            Ok(None)
        } else {
            Err(READ_ERROR.into())
        }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let output = run("secret-tool", "clear", account, "", TIMEOUT)?;
        if output.status.success()
            || (output.status.code() == Some(1) && !output.had_error && output.bytes.is_empty())
        {
            Ok(())
        } else {
            Err(DELETE_ERROR.into())
        }
    }

    pub fn exists(account: &str) -> Result<bool, String> {
        get(account).map(|secret| secret.is_some())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        struct Script(std::path::PathBuf);
        impl Script {
            fn new(body: &str) -> Self {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir().join(format!(
                    "screencue-credential-test-{}-{nonce}",
                    std::process::id()
                ));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .unwrap();
                file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
                    .unwrap();
                file.set_permissions(std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                Self(path)
            }
            fn path(&self) -> &str {
                self.0.to_str().unwrap()
            }
        }
        impl Drop for Script {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        #[test]
        fn child_has_parent_death_signal_and_rejects_changed_parent() {
            let mut command = Command::new("/bin/true");
            protect_parent_exit(&mut command, std::process::id());
            unsafe {
                command.pre_exec(|| {
                    let mut signal: c_int = 0;
                    const PR_GET_PDEATHSIG: c_int = 2;
                    if prctl(PR_GET_PDEATHSIG, &mut signal as *mut c_int) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if signal != SIGKILL {
                        return Err(std::io::Error::from_raw_os_error(4));
                    }
                    Ok(())
                });
            }
            assert!(command.status().unwrap().success());

            let mut changed_parent = Command::new("/bin/true");
            protect_parent_exit(&mut changed_parent, 0);
            assert!(changed_parent.spawn().is_err());
        }

        #[test]
        fn secret_uses_stdin_and_never_command_arguments() {
            let script =
                Script::new("for value do [ \"$value\" != 'owned-test-key' ] || exit 8; done\ncat");
            let output = run(
                script.path(),
                "store",
                "test",
                "owned-test-key",
                Duration::from_secs(2),
            )
            .unwrap();
            assert!(output.status.success());
            assert_eq!(output.bytes, b"owned-test-key");
        }

        #[test]
        fn hung_command_is_reaped_and_errors_do_not_include_diagnostics() {
            let script = Script::new("printf 'owned-secret-diagnostic' >&2\nexec sleep 10");
            let started = Instant::now();
            let error = run(
                script.path(),
                "lookup",
                "test",
                "",
                Duration::from_millis(50),
            )
            .err()
            .unwrap();
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(!error.contains("owned-secret"));
        }

        #[test]
        fn oversized_output_is_rejected_and_missing_executable_is_clear() {
            let script = Script::new("head -c 4096 /dev/zero");
            assert!(run(script.path(), "lookup", "test", "", Duration::from_secs(2)).is_err());
            let error = run(
                "/no-such-screencue-secret-tool",
                "lookup",
                "test",
                "",
                Duration::from_secs(2),
            )
            .err()
            .unwrap();
            assert!(error.contains("secret-tool"));
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod backend {
    pub const NAME: &str = "Unsupported credential store";
    pub fn set(_: &str, _: &str) -> Result<(), String> {
        Err(NAME.into())
    }
    pub fn get(_: &str) -> Result<Option<String>, String> {
        Err(NAME.into())
    }
    pub fn delete(_: &str) -> Result<(), String> {
        Err(NAME.into())
    }
    pub fn exists(_: &str) -> Result<bool, String> {
        Err(NAME.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_before_touching_any_credential_store_and_redacts_errors() {
        for account in ["", "../outside", "space here", "line\nbreak"] {
            assert!(set(account, "owned-test-secret").is_err());
            assert!(get(account).is_err());
            assert!(delete(account).is_err());
            assert!(exists(account).is_err());
        }
        for secret in [
            "",
            " owned-test-secret",
            "owned-test-secret\n",
            "owned\0secret",
        ] {
            let error = set("test", secret).unwrap_err();
            assert!(!error.contains("owned"));
        }
        assert!(validate_account(&"a".repeat(129)).is_err());
        assert!(validate_secret(&"k".repeat(MAX_SECRET + 1)).is_err());
        assert!(validate_secret(&"k".repeat(MAX_SECRET)).is_ok());
        assert!(validate_account("screen-cue.profile_1:fast").is_ok());
    }

    #[test]
    fn corrupt_stored_values_are_not_returned_or_exposed() {
        assert!(decode_secret(&[255]).is_err());
        assert!(decode_secret(b"owned-secret\n").unwrap_err() == READ_ERROR);
        assert!(decode_secret(&vec![b'x'; MAX_SECRET + 1]).is_err());
        assert_eq!(
            decode_secret(b"owned-valid-key").unwrap(),
            "owned-valid-key"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "Explicit opt-in: creates and removes one synthetic Windows credential"]
    fn windows_owned_credential_roundtrip() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let account = format!("test.{}.{nonce}", std::process::id());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        assert!(get(&account).unwrap().is_none());
        assert!(!exists(&account).unwrap());
        let cleanup = Cleanup(account.clone());
        set(&account, "owned-synthetic-key-1").unwrap();
        assert!(exists(&account).unwrap());
        assert!(get(&account).unwrap().as_deref() == Some("owned-synthetic-key-1"));
        set(&account, "owned-synthetic-key-2").unwrap();
        assert!(get(&account).unwrap().as_deref() == Some("owned-synthetic-key-2"));
        delete(&account).unwrap();
        assert!(get(&account).unwrap().is_none());
        assert!(!exists(&account).unwrap());
        delete(&account).unwrap();
        drop(cleanup);
    }
}
