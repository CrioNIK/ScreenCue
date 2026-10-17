# API key storage

ScreenCue stores each provider account's API key separately in the current user's
system credential store. Configuration contains an account identifier, never the
key. There is no plaintext fallback, process-wide environment mutation, shell
interpolation, or key in command arguments. Errors never include keys or raw
credential-store diagnostics. Keys are necessarily present in process memory
while saving, reading, or authenticating a provider request.

| Platform | Backend |
| --- | --- |
| Windows | Generic credentials in Windows Credential Manager, persistent for the current user on this machine |
| macOS | Generic password items in Keychain through native Security.framework APIs |
| Linux | The desktop Secret Service through libsecret's `secret-tool` |

On Linux, install the distribution's libsecret command-line package (`libsecret-tools`
on Debian/Ubuntu, `libsecret` on Fedora/Arch) and enable an unlocked Secret Service
provider such as GNOME Keyring or a compatible desktop keyring. ScreenCue does not
install packages or start a keyring automatically. An absent tool, unavailable
service, denied access or locked keyring produces a visible error. Operations have
a ten-second subprocess deadline; a timed-out process is stopped and reaped.
Keys travel to `secret-tool store` only through its standard input. Standard output
is bounded to the key size and returned to callers only for a successful lookup; diagnostic
text is discarded. No temporary secret files are created.

Credential operations may trigger an operating-system unlock or access prompt.
Call them from a worker thread, not the rendering event loop. macOS controls its
own prompt duration. ScreenCue never overrides OS access controls.

The status API returns only whether an account exists. macOS uses an attributes
query without requesting password data. Windows and Linux perform an internal
lookup and discard the result; Linux may therefore request a keyring unlock even
for a status check. The UI must perform these checks in the background too.

The service namespace is `com.screencue.desktop`; Windows target names append
`:api:<account>`. Account IDs allow 1–128 ASCII letters, digits, dots, underscores,
hyphens and colons. Keys allow 1–2560 UTF-8 bytes without control characters or
surrounding whitespace. The shared size limit matches Windows' generic credential
blob limit. Windows compares target names without case sensitivity; use stable
lowercase account IDs across platforms.

Validation and corrupt-value tests run without reading actual credentials. Linux
subprocess tests use synthetic executable fixtures and never contact a real
keyring. The ignored Windows round-trip test creates a unique synthetic account,
checks creation/update/read/removal, and removes it even if an assertion fails:

```sh
cargo test credentials::tests::windows_owned_credential_roundtrip -- --ignored --exact
```

API references: [Windows CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew),
[CREDENTIALW bounds and persistence](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw),
[Apple SecItemCopyMatching](https://developer.apple.com/documentation/security/secitemcopymatching(_:_:)),
[Apple Keychain update/delete](https://developer.apple.com/documentation/security/updating-and-deleting-keychain-items),
and [GNOME secret-tool source](https://github.com/GNOME/libsecret/blob/main/tool/secret-tool.c).
