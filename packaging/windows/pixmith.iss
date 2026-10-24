; Inno Setup 6 script for the Pixmith Windows installer (egui app + the `pixmith` command line tool).
;
; Build (paths absolute, passed by the CI workflow):
;   ISCC.exe /DAppVersion=0.4.0 /DNumericVersion=0.4.0.0 /DSourceExe=...\pixmith-desktop.exe ^
;            /DSourceCli=...\pixmith-cli.exe /DRepoRoot=... /DOutputDir=... packaging\windows\pixmith.iss
;
; Layout of an install:
;   <app>\Pixmith.exe        the editor
;   <app>\bin\pixmith.exe    the command line tool; only <app>\bin goes on the PATH. It lives in a subfolder because
;                            Windows file names are case-insensitive, so it cannot sit next to Pixmith.exe.
;
; Per-user by default (no admin prompt, %LOCALAPPDATA%\Programs\Pixmith); the first wizard page offers "all users"
; (Program Files, administrator), which then uses the machine PATH and HKLM for the file associations.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; Four-part numeric version for the file properties (AppVersion may be something like 0.0.0-ci).
#ifndef NumericVersion
  #define NumericVersion "0.0.0.0"
#endif
#ifndef RepoRoot
  #define RepoRoot "..\.."
#endif
#ifndef SourceExe
  #define SourceExe RepoRoot + "\desktop\target\release\pixmith-desktop.exe"
#endif
#ifndef SourceCli
  #define SourceCli RepoRoot + "\core\target\release\pixmith-cli.exe"
#endif
#ifndef OutputDir
  #define OutputDir RepoRoot + "\dist"
#endif

#define AppName "Pixmith"
#define AppExe "Pixmith.exe"
#define Publisher "Azdhar Syahputra"
#define AppUrl "https://github.com/azdharsyahputra/pixmith"

[Setup]
; Fixed forever: this is what makes a newer installer upgrade the existing install in place.
AppId={{FC7C9AB6-A745-47D8-9E84-E920FF36ECC1}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#Publisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppCopyright=Copyright (c) Pixmith contributors. MIT License.
VersionInfoVersion={#NumericVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
VersionInfoCompany={#Publisher}

PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; {autopf} is %LOCALAPPDATA%\Programs for a per-user install and Program Files for an all-users one.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

LicenseFile={#RepoRoot}\LICENSE
SetupIconFile={#RepoRoot}\desktop\assets\pixmith.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes

OutputDir={#OutputDir}
OutputBaseFilename=Pixmith-Setup-{#AppVersion}-x64

; Close a running Pixmith (or pixmith.exe) before replacing its files, and leave it closed.
CloseApplications=yes
RestartApplications=no
; Registers the file associations and the PATH change with the shell.
ChangesAssociations=yes
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath"; Description: "Add the &pixmith command to the PATH"; GroupDescription: "Command line tool:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "associateaseprite"; Description: "Open &.aseprite and .ase files with Pixmith"; GroupDescription: "File types:"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "{#SourceCli}"; DestDir: "{app}\bin"; DestName: "pixmith.exe"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\desktop\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Registry]
; HKA is HKCU for a per-user install and HKLM for an all-users one. Everything below is removed on uninstall.

; .pxmt belongs to Pixmith.
Root: HKA; Subkey: "Software\Classes\.pxmt"; ValueType: string; ValueName: ""; ValueData: "Pixmith.Document"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.pxmt\OpenWithProgids"; ValueType: string; ValueName: "Pixmith.Document"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\Pixmith.Document"; ValueType: string; ValueName: ""; ValueData: "Pixmith Document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Pixmith.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\Pixmith.Document\shell"; ValueType: string; ValueName: ""; ValueData: "open"
Root: HKA; Subkey: "Software\Classes\Pixmith.Document\shell\open"; ValueType: string; ValueName: ""; ValueData: "Open with Pixmith"
Root: HKA; Subkey: "Software\Classes\Pixmith.Document\shell\open"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\Pixmith.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

; Optional task: .aseprite / .ase open with Pixmith by default (own ProgID, so Explorer says "Aseprite Document").
Root: HKA; Subkey: "Software\Classes\.aseprite"; ValueType: string; ValueName: ""; ValueData: "Pixmith.AsepriteDocument"; Flags: uninsdeletevalue uninsdeletekeyifempty; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\.ase"; ValueType: string; ValueName: ""; ValueData: "Pixmith.AsepriteDocument"; Flags: uninsdeletevalue uninsdeletekeyifempty; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument"; ValueType: string; ValueName: ""; ValueData: "Aseprite Document"; Flags: uninsdeletekey; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument\shell"; ValueType: string; ValueName: ""; ValueData: "open"; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument\shell\open"; ValueType: string; ValueName: ""; ValueData: "Open with Pixmith"; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument\shell\open"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: associateaseprite
Root: HKA; Subkey: "Software\Classes\Pixmith.AsepriteDocument\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: associateaseprite

; Always: list Pixmith under "Open with" for the supported types, without taking them over.
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".pxmt"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".aseprite"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".ase"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.aseprite\OpenWithProgids"; ValueType: string; ValueName: "Pixmith.AsepriteDocument"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.ase\OpenWithProgids"; ValueType: string; ValueName: "Pixmith.AsepriteDocument"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty

[Code]
const
  UserEnvKey = 'Environment';
  MachineEnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  PX_WM_SETTINGCHANGE = $001A;
  PX_HWND_BROADCAST = $FFFF;
  PX_SMTO_ABORTIFHUNG = $0002;

function SendMessageTimeout(hWnd: LongWord; Msg: LongWord; wParam: LongWord; lParam: String;
  fuFlags: LongWord; uTimeout: LongWord; out lpdwResult: LongWord): LongWord;
  external 'SendMessageTimeoutW@user32.dll stdcall';

// Tell running programs (Explorer, new terminals) that the environment changed.
procedure BroadcastEnvironmentChange;
var
  Res: LongWord;
begin
  SendMessageTimeout(PX_HWND_BROADCAST, PX_WM_SETTINGCHANGE, 0, 'Environment', PX_SMTO_ABORTIFHUNG, 5000, Res);
end;

// The per-user PATH for a current-user install, the machine PATH for an all-users one.
function EnvRoot: Integer;
begin
  if IsAdminInstallMode then Result := HKEY_LOCAL_MACHINE else Result := HKEY_CURRENT_USER;
end;

function EnvSubkey: String;
begin
  if IsAdminInstallMode then Result := MachineEnvKey else Result := UserEnvKey;
end;

// Compare PATH entries without caring about case, quotes or trailing backslashes.
function NormalizeEntry(S: String): String;
begin
  S := Trim(S);
  if (Length(S) >= 2) and (S[1] = '"') and (S[Length(S)] = '"') then
    S := Trim(Copy(S, 2, Length(S) - 2));
  while (Length(S) > 3) and (S[Length(S)] = '\') do
    Delete(S, Length(S), 1);
  Result := S;
end;

function SameEntry(const A, B: String): Boolean;
begin
  Result := CompareText(NormalizeEntry(A), NormalizeEntry(B)) = 0;
end;

// Returns the ';' separated list without empty entries and without entries equal to Dir. Found says whether any
// entry was equal to Dir.
function StripEntry(const Path, Dir: String; out Found: Boolean): String;
var
  Rest, Entry: String;
  P: Integer;
begin
  Found := False;
  Result := '';
  Rest := Path;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Entry := Rest;
      Rest := '';
    end
    else
    begin
      Entry := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, Length(Rest));
    end;
    if Trim(Entry) = '' then
      Continue;
    if SameEntry(Entry, Dir) then
      Found := True
    else if Result = '' then
      Result := Entry
    else
      Result := Result + ';' + Entry;
  end;
end;

function ReadPath: String;
begin
  if not RegQueryStringValue(EnvRoot, EnvSubkey, 'Path', Result) then
    Result := '';
end;

procedure WritePath(const Value: String);
begin
  // REG_EXPAND_SZ keeps entries such as %SystemRoot%\system32 working.
  if not RegWriteExpandStringValue(EnvRoot, EnvSubkey, 'Path', Value) then
    Log('Could not write the PATH value');
end;

procedure AddToPath(const Dir: String);
var
  Cleaned: String;
  Found: Boolean;
begin
  Cleaned := StripEntry(ReadPath, Dir, Found);
  if Found then
  begin
    Log('PATH already contains ' + Dir);
    Exit;
  end;
  if Cleaned = '' then
    WritePath(Dir)
  else
    WritePath(Cleaned + ';' + Dir);
  Log('Added to PATH: ' + Dir);
  BroadcastEnvironmentChange;
end;

procedure RemoveFromPath(const Dir: String);
var
  Cleaned: String;
  Found: Boolean;
begin
  Cleaned := StripEntry(ReadPath, Dir, Found);
  if not Found then
    Exit;
  WritePath(Cleaned);
  Log('Removed from PATH: ' + Dir);
  BroadcastEnvironmentChange;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToPath(ExpandConstant('{app}\bin'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}\bin'));
end;
