# Capture privacy

ScreenCue can ask supported Windows capture APIs to leave its windows out of a
screen recording while keeping them visible on the local monitor. The setting
applies separately to the hint overlay and the settings window. It does not hide
the process, change another application's capture settings, or establish that a
remote participant cannot see the hints.

## Platform support

| Platform | ScreenCue behavior | Limitation |
| --- | --- | --- |
| Windows 10 version 2004 / build 19041 and newer | Apply `WDA_EXCLUDEFROMCAPTURE` to each owned top-level window; read it back with `GetWindowDisplayAffinity`. | Capture software must honor the operating-system policy. Validate the selected capture source and the receiver's actual view. |
| Older Windows | Report exclusion as unavailable. | `0x11` becomes `WDA_MONITOR` on older versions, which can produce a blank captured window. ScreenCue does not silently substitute that behavior. |
| macOS | Report exclusion as unavailable. | `NSWindow.sharingType = .none` is not reliable protection against ScreenCaptureKit. ScreenCue does not label that legacy flag as privacy. |
| Linux / X11 | Report exclusion as unavailable. | The current backend has no supported per-window opt-out from other applications' capture. |
| Linux / Wayland | No exclusion backend is implemented. | Screen-cast source selection belongs to the portal/compositor and the capturing application; there is no supported generic ScreenCue window opt-out in this implementation. |

On unsupported systems, sharing a particular application window instead of a
whole monitor may keep the separate hint window out of that source. This is a
capture-source choice, not a ScreenCue guarantee. Check the sharing preview and,
where possible, the receiver's view using the exact application and mode you
will use.

## Native contract

`platform::capture_privacy_capability()` reports whether the operating system
supports ScreenCue's exclusion backend without changing any window.

`platform::set_capture_exclusion(window, requested)` applies or removes the
policy for one window. Call it after constructing the native presenter and
before making that window visible. Newly created windows need their own call.
The operation does not map, hide, activate, or resize a window.

The serializable `CapturePrivacyStatus` contains:

| Field | Meaning |
| --- | --- |
| `requested` | Whether exclusion was requested for this window. |
| `supported` | Whether this operating system supports the implemented exclusion backend. This alone does not mean a policy was applied. |
| `applied` | Whether exclusion is currently confirmed by native policy readback. False also covers an unknown state after a read failure. |
| `verified` | Whether applying the requested policy succeeded and native readback matched it. This can be true while disabling exclusion. It does **not** verify a sharing application's output. |
| `backend` | `win32_display_affinity` or `unsupported`. |
| `capture_guaranteed` | Always false. No universal capture guarantee is claimed. |
| `error` | Native failure, unsupported request, or readback mismatch; null on successful application. |
| `note` | The remaining limitation to show alongside the setting. |

Windows checks the real OS version with `RtlGetVersion` before applying `0x11`.
A successful `GetWindowDisplayAffinity` also confirms the conditions required by
that API, including DWM composition. The implementation does not use a false
result from `DwmIsCompositionEnabled` as its sole capability test: that API can
report false for a composed desktop when the executable lacks a Windows 8
compatibility manifest.

Changing or disabling privacy does not clear hint text. If a caller must avoid
an unprotected frame, it must keep the window hidden until `applied` and
`verified` are both true. A stored preference by itself is not evidence of an
applied policy, and a previous success is not a fresh capture verification.

## Validation scope

The native Rust test creates its own hidden layered top-level HWND, enables
exclusion, reads back exactly `0x11`, disables it, and reads back exactly `0x0`.
It also checks that visibility and keyboard focus did not change. Additional
tests cover the Windows build boundary, unsupported behavior, and native
failure reporting. These checks passed on the development Windows host.

`tests/windows-app.ps1` also launches the real settings application and a JSON
overlay connection using isolated test preferences. It verifies the settings
window's icon, visibility and close behavior; native `0x11`/`0x0` readback on
both window types; the standalone conversation prompt's status; no overlay focus
change; audio remaining stopped; and clean process shutdown. Both preference
states passed on the development Windows host.

This test does not record Zoom, Teams, Meet, OBS, browser sharing, a game,
remote desktop, or an external capture device. Compatibility with those capture
paths is not implied by the test. No microphone, meeting audio, screen recording,
or model service is started by the capture-privacy setting.

## Primary references

- Microsoft: [SetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)
  documents the supported version, current-process top-level window requirement,
  legacy fallback, and limits of the policy.
- Microsoft: [GetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity)
  describes policy readback and its layered-window / DWM requirements.
- Microsoft: [DwmIsCompositionEnabled](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmiscompositionenabled)
  describes the compatibility-manifest caveat.
- Microsoft: [RtlGetVersion](https://learn.microsoft.com/en-us/windows/win32/devnotes/rtlgetversion)
  documents native OS version reporting.
- Apple DTS: [ScreenCaptureKit and NSWindowSharingNone](https://developer.apple.com/forums/thread/792152)
  states that no public API prevents this capture. The
  [capture sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)
  shows that exclusion filters belong to the capturing application.
- XDG Desktop Portal: [ScreenCast](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
  documents monitor/window source selection by the capture session.

References checked on 2026-10-17.
