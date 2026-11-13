; Inno Setup 6 script: the Windows setup program of Pandite.
;
; Build it with packaging/windows/build-setup.ps1, which passes:
;   /DVersion=<x.y.z>          release version (Cargo.toml)
;   /DNumericVersion=<x.y.z.0> the same version as four numbers
;   /DTarget=<rust target>     x86_64-pc-windows-msvc or aarch64-pc-windows-msvc
;   /DArch=<x64|arm64>         the architecture of the program
;   /DExe=<path>               the release pandite.exe
;   /DOutDir=<dir>             output folder
;
; The setup program installs the files of the portable zip for the current
; user, without administrator rights. The "associate" task runs
; `pandite.exe --register`; uninstall runs `pandite.exe --unregister`.
; Before an upgrade or an uninstall, `pandite.exe --quit` ends the pandite
; that waits in the background, so that it does not lock pandite.exe.

#if VER < EncodeVer(6,3,0)
  #error Inno Setup 6.3 or later is required (x64compatible architecture name)
#endif
#ifndef Version
  #error Pass /DVersion=<x.y.z>, or build with packaging/windows/build-setup.ps1
#endif
#ifndef NumericVersion
  #error Pass /DNumericVersion=<x.y.z.0>, or build with packaging/windows/build-setup.ps1
#endif
#ifndef Target
  #error Pass /DTarget=<rust target>, or build with packaging/windows/build-setup.ps1
#endif
#ifndef Arch
  #error Pass /DArch=x64 or /DArch=arm64, or build with packaging/windows/build-setup.ps1
#endif
#ifndef Exe
  #error Pass /DExe=<path to pandite.exe>, or build with packaging/windows/build-setup.ps1
#endif
#ifndef OutDir
  #error Pass /DOutDir=<output folder>, or build with packaging/windows/build-setup.ps1
#endif

[Setup]
; The AppId identifies the installation for upgrades and uninstall. Never change it.
AppId={{E5E712AA-D4BF-4DC9-BB19-9841DC38CD1E}
AppName=Pandite
AppVersion={#Version}
AppPublisher=Thomas Rizos
AppPublisherURL=https://github.com/xls/pandite
AppSupportURL=https://github.com/xls/pandite/issues
AppUpdatesURL=https://github.com/xls/pandite/releases
VersionInfoVersion={#NumericVersion}
VersionInfoProductName=Pandite
VersionInfoProductTextVersion={#Version}
VersionInfoDescription=Pandite Setup
; Per-user install: no administrator rights, no elevation prompt.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Pandite
DisableProgramGroupPage=yes
DisableDirPage=auto
UninstallDisplayName=Pandite
UninstallDisplayIcon={app}\pandite.exe
SetupIconFile=..\..\crates\pandite-app\assets\pandite.ico
LicenseFile=..\..\LICENSE
; Same floor as the MSIX package (AppxManifest.xml TargetDeviceFamily).
MinVersion=10.0.17763
#if Arch == "x64"
; x64 Windows, and Windows 11 on ARM64 through x64 emulation.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#elif Arch == "arm64"
; ARM64 Windows only.
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
  #error Arch must be x64 or arm64
#endif
ChangesAssociations=yes
; The Restart Manager closes a pandite window that holds pandite.exe open.
CloseApplications=yes
RestartApplications=no
WizardStyle=modern
ShowLanguageDialog=no
Compression=lzma2/max
SolidCompression=yes
SetupLogging=yes
OutputDir={#OutDir}
OutputBaseFilename=pandite-{#Version}-{#Target}-setup

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "associate"; Description: "Associate image files with Pandite"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The files of the portable zip (packaging/windows/build-zip.ps1).
Source: "{#Exe}"; DestDir: "{app}"; DestName: "pandite.exe"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion
Source: "..\..\docs\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.md"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Pandite"; Filename: "{app}\pandite.exe"; Comment: "Fast offline image viewer"
Name: "{autodesktop}\Pandite"; Filename: "{app}\pandite.exe"; Comment: "Fast offline image viewer"; Tasks: desktopicon

[Run]
Filename: "{app}\pandite.exe"; Parameters: "--register"; StatusMsg: "Associating image files with Pandite..."; Tasks: associate; Flags: runhidden waituntilterminated
Filename: "{app}\pandite.exe"; Description: "{cm:LaunchProgram,Pandite}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\pandite.exe"; Parameters: "--unregister"; RunOnceId: "UnregisterFileTypes"; Flags: runhidden waituntilterminated

[Code]
const
  QuitTimeoutMs = 8000;
  QuitPollMs = 250;

{ True while a process runs from the program file Exe. WMI lists the
  processes of this user without administrator rights. }
function ProgramRuns(const Exe: String): Boolean;
var
  Locator, Service, Found: Variant;
  Path: String;
begin
  Result := False;
  Path := Exe;
  { WQL strings escape the backslash. }
  StringChangeEx(Path, '\', '\\', True);
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Found := Service.ExecQuery('SELECT ProcessId FROM Win32_Process WHERE ExecutablePath = "' + Path + '"');
    Result := Found.Count > 0;
  except
    Log('Cannot list the running processes: ' + GetExceptionMessage);
    Result := False;
  end;
end;

{ Run "pandite.exe --quit" from the installed folder, then wait until no
  process runs from that file, so that Setup can replace or delete it. }
procedure QuitResidentPandite;
var
  Exe: String;
  ResultCode, Waited: Integer;
begin
  Exe := ExpandConstant('{app}\pandite.exe');
  if not FileExists(Exe) then
    Exit;
  Log('Asking the resident Pandite to exit: ' + Exe + ' --quit');
  if not Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    Log('Cannot run pandite.exe --quit: ' + SysErrorMessage(ResultCode));
  Waited := 0;
  while ProgramRuns(Exe) and (Waited < QuitTimeoutMs) do
  begin
    Sleep(QuitPollMs);
    Waited := Waited + QuitPollMs;
  end;
  if Waited >= QuitTimeoutMs then
    Log('pandite.exe still runs; the Restart Manager closes it');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  QuitResidentPandite;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    QuitResidentPandite;
end;
