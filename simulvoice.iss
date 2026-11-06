[Setup]
AppId={{8F3B2E54-6C1D-4B7A-9E2F-3A5C7D1B4E60}
AppName=SimulVoice
AppVersion=0.45.0
AppVerName=SimulVoice 0.45.0
AppPublisher=SimulVoice
DefaultDirName={localappdata}\Programs\SimulVoice
DefaultGroupName=SimulVoice
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=dist
OutputBaseFilename=SimulVoiceSetup-0.45.0
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\simulvoice.exe
SetupIconFile=dist\stage\simulvoice.ico
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "chs"; MessagesFile: "installer\ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[InstallDelete]
; Before 0.45.0 the uninstall shortcut had an English name; remove it after the rename
Type: files; Name: "{group}\Uninstall SimulVoice.lnk"

[Files]
Source: "dist\stage\simulvoice.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\stage\onnxruntime_marian.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "dist\stage\README.md"; DestDir: "{app}"
Source: "dist\stage\simulvoice.data-dir"; DestDir: "{app}"; Flags: onlyifdoesntexist

[Icons]
Name: "{group}\SimulVoice"; Filename: "{app}\simulvoice.exe"
Name: "{group}\{cm:UninstallProgram,SimulVoice}"; Filename: "{uninstallexe}"; IconFilename: "{sys}\shell32.dll"; IconIndex: 31
Name: "{autodesktop}\SimulVoice"; Filename: "{app}\simulvoice.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\simulvoice.exe"; Description: "{cm:LaunchProgram,SimulVoice}"; Flags: nowait postinstall skipifsilent

[CustomMessages]
chs.RemoveData=是否同时删除 SimulVoice 的数据？%n%n包括设置、密钥、下载的语音和翻译模型、录音和日志，位置：%n%1%n%n以后还要重新安装的话，选「否」可以保留，省去重新下载模型。
en.RemoveData=Also delete SimulVoice data?%n%nSettings, API keys, downloaded speech/translation models, recordings and logs in:%n%1%n%nChoose No to keep them if you plan to reinstall.

[Code]
var
  DataDir: String;

{ 数据目录：exe 旁的 simulvoice.data-dir 里写的是相对路径（data）或绝对路径 }
function ResolveDataDir(): String;
var
  Raw: AnsiString;
  P: String;
begin
  Result := ExpandConstant('{app}\data');
  if LoadStringFromFile(ExpandConstant('{app}\simulvoice.data-dir'), Raw) then
  begin
    P := Trim(String(Raw));
    if P <> '' then
    begin
      if (Pos(':', P) > 0) or (Copy(P, 1, 2) = '\\') then
        Result := P
      else
        Result := ExpandConstant('{app}\') + P;
    end;
  end;
end;

{ 只删 SimulVoice 自己生成的东西：数据目录可能被改到用户自己的文件夹，不能整个删 }
procedure DeleteDataItems(Dir: String);
begin
  DelTree(Dir + '\models', True, True, True);
  DelTree(Dir + '\rec', True, True, True);
  DelTree(Dir + '\recordings', True, True, True);
  DelTree(Dir + '\crashdumps', True, True, True);
  DeleteFile(Dir + '\simulvoice.toml');
  DeleteFile(Dir + '\keys.toml');
  DeleteFile(Dir + '\simulvoice.log');
  DeleteFile(Dir + '\transcript.txt');
  DeleteFile(Dir + '\update-check.txt');
  RemoveDir(Dir);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { 标记文件马上会被卸载掉，先读出数据目录 }
  if CurUninstallStep = usUninstall then
    DataDir := ResolveDataDir();
  if (CurUninstallStep = usPostUninstall) and (DataDir <> '') and DirExists(DataDir) and (not UninstallSilent()) then
  begin
    if MsgBox(FmtMessage(CustomMessage('RemoveData'), [DataDir]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DeleteDataItems(DataDir);
      RemoveDir(ExpandConstant('{app}'));
    end;
  end;
end;
