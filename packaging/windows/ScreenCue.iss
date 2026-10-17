#ifndef AppVersion
  #define AppVersion "0.0.4"
#endif
#ifndef SourceDir
  #error SourceDir must point to the staged Windows application directory.
#endif
#ifndef OutputPath
  #define OutputPath "..\..\dist\app"
#endif
#ifndef IconPath
  #define IconPath "..\..\assets\app-icon.ico"
#endif

[Setup]
AppId={{B8D1300B-19EC-4C4E-91D3-3113761584A1}
AppName=ScreenCue
AppVersion={#AppVersion}
AppPublisher=ScreenCue contributors
AppPublisherURL=https://github.com/CrioNIK/ScreenCue
AppSupportURL=https://github.com/CrioNIK/ScreenCue/issues
AppUpdatesURL=https://github.com/CrioNIK/ScreenCue/releases
DefaultDirName={localappdata}\Programs\ScreenCue
DefaultGroupName=ScreenCue
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
WizardStyle=modern dark
WizardSizePercent=110
SetupIconFile={#IconPath}
WizardSmallImageFile={#SourceDir}\app-icon.png
UninstallDisplayIcon={app}\screencue.exe
UninstallDisplayName=ScreenCue
LicenseFile={#SourceDir}\LICENSE
OutputDir={#OutputPath}
OutputBaseFilename=ScreenCue-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
CloseApplicationsFilter=screencue.exe,screencue-listen.exe
RestartApplications=no
DisableWelcomePage=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel1=Your next words. Right where you need them.
WelcomeLabel2=ScreenCue adds private notes and visual guidance to your desktop.%n%nChoose your model, connect an agent, and keep your attention on the conversation.%n%nAudio starts only when you turn it on. No account or administrator access is required to install.
FinishedHeadingLabel=ScreenCue is ready
FinishedLabel=Open ScreenCue to set up your hints and optional conversation assistant.

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\ScreenCue"; Filename: "{app}\screencue.exe"; Parameters: "--app"; WorkingDir: "{app}"
Name: "{group}\Uninstall ScreenCue"; Filename: "{uninstallexe}"
Name: "{autodesktop}\ScreenCue"; Filename: "{app}\screencue.exe"; Parameters: "--app"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\screencue.exe"; Parameters: "--app"; Description: "Open ScreenCue"; Flags: nowait postinstall skipifsilent
