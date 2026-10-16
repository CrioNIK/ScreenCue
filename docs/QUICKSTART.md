# ScreenCue 0.1.0

Подсказки от ИИ поверх рабочего стола. Один исполняемый файл, без установки Rust, Node.js или .NET.

1. Сохраните распакованную папку в постоянном месте.
2. Запустите `screencue demo` (`.\screencue.exe demo` в Windows PowerShell). Убрать подсказку: **Ctrl+Alt+X**, на Mac — **Ctrl+Option+X**.
3. Подключите программу к своему агенту через локальный MCP. Например, для Codex на Windows:

```powershell
codex mcp add screencue -- "C:\Tools\ScreenCue\screencue.exe" --mcp
```

Замените путь своим; на macOS/Linux используйте абсолютный путь к `screencue`. Откройте новую сессию клиента. [Подключение Codex, Claude Code, Gemini CLI и других агентов](https://github.com/CrioNIK/ScreenCue/blob/main/docs/INTEGRATION.md).

Стрелка пропускает клики, не забирает фокус и следует за окном. За экраном или при сворачивании остаётся пометка у края. Прокрутка, изменение размера и перестройка интерфейса требуют свежих координат от агента. Голос и наблюдение за экраном обеспечивает клиент; ScreenCue рисует полученную подсказку.

Windows x64; macOS 12.3+ (автоцвет по экрану — 15.2+ с уже выданным разрешением); Linux x64/X11 с композитором. Native Wayland пока не поддерживается. Выпуск предварительный; поведение живого окна проверено на Windows, для macOS/Linux выполнены сборки и модульные тесты.

[Исходники, параметры и ограничения](https://github.com/CrioNIK/ScreenCue). Код: MIT (`LICENSE`), встроенный шрифт IBM Plex Mono: SIL OFL (`licenses/IBM-Plex-Mono-OFL.txt`).
