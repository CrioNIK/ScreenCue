# Font provenance

ScreenCue includes unmodified static IBM Plex Sans fonts for native hint text. They are embedded assets, not installed system fonts. No font package manager or network request is needed at runtime.

## IBM Plex Sans

Source: [IBM's official Plex repository](https://github.com/IBM/plex), pinned to commit [`71f558992315a12ca1f06a2215d57962a1992162`](https://github.com/IBM/plex/commit/71f558992315a12ca1f06a2215d57962a1992162). Retrieved October 17, 2026.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `assets/IBMPlexSans-Regular.ttf` | 200,500 | `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5` |
| `assets/IBMPlexSans-Bold.ttf` | 200,872 | `9e6c74a889a700d707613d24548fe4ffa6bc59559a0689d2cf9e133bdcdafb2f` |

Exact upstream files:

- [Regular TTF](https://github.com/IBM/plex/blob/71f558992315a12ca1f06a2215d57962a1992162/packages/plex-sans/fonts/complete/ttf/IBMPlexSans-Regular.ttf)
- [Bold TTF](https://github.com/IBM/plex/blob/71f558992315a12ca1f06a2215d57962a1992162/packages/plex-sans/fonts/complete/ttf/IBMPlexSans-Bold.ttf)
- [Upstream license](https://github.com/IBM/plex/blob/71f558992315a12ca1f06a2215d57962a1992162/packages/plex-sans/LICENSE.txt)

Both files contain static TrueType outlines and 894 mapped Unicode code points. A binary `cmap` check verified printable ASCII, the basic Cyrillic alphabet, and the additional Ukrainian letters at U+0404/U+0454, U+0406/U+0456, U+0407/U+0457, and U+0490/U+0491. This is a coverage check for those characters, not a claim of universal language or shaping support.

The original files have not been subsetted, renamed, or regenerated. Their combined size is 401,372 bytes (approximately 392 KiB).

## License

Copyright 2017 IBM Corp.; Reserved Font Name: Plex. Licensed under **SIL Open Font License 1.1** (`OFL-1.1`). The complete upstream license is already included as [assets/OFL.txt](../assets/OFL.txt) and is byte-identical to the Sans package's license at the pinned commit.

License SHA-256: `7e6b2818edbd8f6a01ae80641cc8f16a51080d08fb4e532be3a0b6f74adb07da`.

Keep this license with redistributed binaries or font assets. The existing IBM Plex Mono asset is covered by the same notice. Some earlier release archives name the copy `IBM-Plex-Mono-OFL.txt`; its contents apply to these IBM Plex Sans fonts as well. For newly named notices, `IBM-Plex-OFL.txt` avoids implying that it covers only Mono.

Any later subset or modified font must comply with the license's Reserved Font Name condition. This import avoids that issue by preserving IBM's complete original files.
