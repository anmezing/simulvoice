[Setup]
AppId={{8F3B2E54-6C1D-4B7A-9E2F-3A5C7D1B4E60}
AppName=SimulVoice
AppVersion=0.14.1
AppVerName=SimulVoice 0.14.1
AppPublisher=SimulVoice
DefaultDirName={localappdata}\Programs\SimulVoice
DefaultGroupName=SimulVoice
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=dist
OutputBaseFilename=SimulVoiceSetup-0.14.1
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\simulvoice.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "chs"; MessagesFile: "installer\ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "dist\stage\simulvoice.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\stage\README.md"; DestDir: "{app}"
Source: "dist\stage\simulvoice.toml"; DestDir: "{app}"
Source: "dist\stage\simulvoice.data-dir"; DestDir: "{app}"

[Icons]
Name: "{group}\SimulVoice"; Filename: "{app}\simulvoice.exe"
Name: "{group}\Uninstall SimulVoice"; Filename: "{uninstallexe}"
Name: "{autodesktop}\SimulVoice"; Filename: "{app}\simulvoice.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\simulvoice.exe"; Description: "{cm:LaunchProgram,SimulVoice}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\data"
Type: files; Name: "{app}\simulvoice.data-dir"




