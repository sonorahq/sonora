#define AppName "Sonora"
#define AppPublisher "Sonora"
#define AppExeName "sonora.exe"
#define AppVersion GetEnv("SONORA_VERSION")
#define SourceExe GetEnv("SONORA_EXE")
#define OutputDir GetEnv("SONORA_DIST")
; x64compatible for the x64 build, arm64 for the ARM one
#define Arch GetEnv("SONORA_ARCH")
; Sonora-Setup for the x64 build, Sonora-Setup-arm64 for the ARM one
#define SetupName GetEnv("SONORA_SETUP")

[Setup]
AppId={{8D65C17E-79E8-46D7-9A37-42E85E73F738}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir={#OutputDir}
OutputBaseFilename={#SetupName}
SetupIconFile=..\..\assets\windows\sonora.ico
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
WizardStyle=modern
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
Source: "..\..\COPYING"; DestDir: "{app}"; DestName: "LICENSE"; Flags: ignoreversion
Source: "..\..\THIRD-PARTY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon; Check: not SilentUpgrade

; Lists Sonora in "Open With" and in Settings > Apps > Default apps for the file types
; it plays, without automatically becoming the default handler for any of them.
; MultiSelectModel=Player is the key Explorer honors to invoke the app once
; with every selected file passed as its own argument, instead of once per file.
[Registry]
Root: HKA; Subkey: "Software\Sonora"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Sonora\Capabilities"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Sonora\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "A native music streaming client, built with Rust and GPUI"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: "{app}\{#AppExeName},0"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\Sonora\Capabilities"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
; .mp3
Root: HKA; Subkey: "Software\Classes\.mp3"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: string; ValueName: "Sonora.mp3"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.mp3"; ValueType: string; ValueName: ""; ValueData: "MP3 Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.mp3"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.mp3\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.mp3\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mp3"; ValueData: "Sonora.mp3"; Flags: uninsdeletevalue
; .flac
Root: HKA; Subkey: "Software\Classes\.flac"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.flac\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.flac\OpenWithProgids"; ValueType: string; ValueName: "Sonora.flac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.flac"; ValueType: string; ValueName: ""; ValueData: "FLAC Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.flac"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.flac\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.flac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".flac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".flac"; ValueData: "Sonora.flac"; Flags: uninsdeletevalue
; .m4a
Root: HKA; Subkey: "Software\Classes\.m4a"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m4a\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.m4a\OpenWithProgids"; ValueType: string; ValueName: "Sonora.m4a"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.m4a"; ValueType: string; ValueName: ""; ValueData: "MPEG-4 Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.m4a"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.m4a\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.m4a\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".m4a"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m4a"; ValueData: "Sonora.m4a"; Flags: uninsdeletevalue
; .mp4 (no default apps entry, since most .mp4 files are videos)
Root: HKA; Subkey: "Software\Classes\.mp4"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithProgids"; ValueType: string; ValueName: "Sonora.mp4"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.mp4"; ValueType: string; ValueName: ""; ValueData: "MPEG-4 Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.mp4"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.mp4\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.mp4\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""; Flags: uninsdeletevalue
; .aac
Root: HKA; Subkey: "Software\Classes\.aac"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aac\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.aac\OpenWithProgids"; ValueType: string; ValueName: "Sonora.aac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.aac"; ValueType: string; ValueName: ""; ValueData: "AAC Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.aac"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.aac\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.aac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".aac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".aac"; ValueData: "Sonora.aac"; Flags: uninsdeletevalue
; .ogg
Root: HKA; Subkey: "Software\Classes\.ogg"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ogg\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ogg\OpenWithProgids"; ValueType: string; ValueName: "Sonora.ogg"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.ogg"; ValueType: string; ValueName: ""; ValueData: "Ogg Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.ogg"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.ogg\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.ogg\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".ogg"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ogg"; ValueData: "Sonora.ogg"; Flags: uninsdeletevalue
; .oga
Root: HKA; Subkey: "Software\Classes\.oga"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.oga\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.oga\OpenWithProgids"; ValueType: string; ValueName: "Sonora.oga"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.oga"; ValueType: string; ValueName: ""; ValueData: "Ogg Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.oga"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.oga\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.oga\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".oga"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".oga"; ValueData: "Sonora.oga"; Flags: uninsdeletevalue
; .opus
Root: HKA; Subkey: "Software\Classes\.opus"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.opus\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.opus\OpenWithProgids"; ValueType: string; ValueName: "Sonora.opus"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.opus"; ValueType: string; ValueName: ""; ValueData: "Opus Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.opus"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.opus\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.opus\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".opus"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".opus"; ValueData: "Sonora.opus"; Flags: uninsdeletevalue
; .wav
Root: HKA; Subkey: "Software\Classes\.wav"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithProgids"; ValueType: string; ValueName: "Sonora.wav"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.wav"; ValueType: string; ValueName: ""; ValueData: "WAVE Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.wav"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.wav\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.wav\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".wav"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wav"; ValueData: "Sonora.wav"; Flags: uninsdeletevalue
; .webm
Root: HKA; Subkey: "Software\Classes\.webm"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithProgids"; ValueType: string; ValueName: "Sonora.webm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.webm"; ValueType: string; ValueName: ""; ValueData: "WebM Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.webm"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.webm\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.webm\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".webm"; ValueData: "Sonora.webm"; Flags: uninsdeletevalue
; .mka
Root: HKA; Subkey: "Software\Classes\.mka"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mka\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.mka\OpenWithProgids"; ValueType: string; ValueName: "Sonora.mka"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.mka"; ValueType: string; ValueName: ""; ValueData: "Matroska Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.mka"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.mka\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.mka\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".mka"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mka"; ValueData: "Sonora.mka"; Flags: uninsdeletevalue
; .wv
Root: HKA; Subkey: "Software\Classes\.wv"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wv\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.wv\OpenWithProgids"; ValueType: string; ValueName: "Sonora.wv"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.wv"; ValueType: string; ValueName: ""; ValueData: "WavPack Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.wv"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.wv\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.wv\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".wv"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wv"; ValueData: "Sonora.wv"; Flags: uninsdeletevalue
; .ape
Root: HKA; Subkey: "Software\Classes\.ape"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ape\OpenWithProgids"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ape\OpenWithProgids"; ValueType: string; ValueName: "Sonora.ape"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Sonora.ape"; ValueType: string; ValueName: ""; ValueData: "Monkey's Audio"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Sonora.ape"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"
Root: HKA; Subkey: "Software\Classes\Sonora.ape\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\Sonora.ape\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".ape"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Sonora\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ape"; ValueData: "Sonora.ape"; Flags: uninsdeletevalue
; spotify
Root: HKA; Subkey: "Software\Sonora\Capabilities\UrlAssociations"; ValueType: string; ValueName: "spotify"; ValueData: "SonoraSpotify"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\SonoraSpotify"; ValueType: string; ValueName: ""; ValueData: "Spotify Link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\SonoraSpotify"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\SonoraSpotify\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\SonoraSpotify\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""

; Setup runs elevated, and a [Run] entry inherits that unless it says otherwise: postinstall
; entries default to runasoriginaluser, the relaunch after a silent update does not, and an
; elevated Sonora is out of reach for tools like FancyZones that manage windows unelevated.
[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\{#AppExeName}"; Flags: nowait runasoriginaluser; Check: RelaunchRequested

[Code]
function RelaunchRequested: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

// A silent run over an existing install is an update from the app or a package manager. It
// leaves the desktop alone, so a shortcut the user deleted or replaced stays that way.
function SilentUpgrade: Boolean;
begin
  Result := WizardSilent and (WizardForm.PrevAppDir <> '');
end;
