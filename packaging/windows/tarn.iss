; Per-user installer for the folder scripts/package-windows.sh makes. Compile with
; scripts/installer-windows.sh, or ISCC /DAppVersion=1.2.3 /DPackageDir=<folder> /DOutputDir=<dir>;
; scripts/sign-windows.ps1 adds /DSign and a sign tool named "tarn".
#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef PackageDir
  #error Pass /DPackageDir=<folder made by scripts/package-windows.sh>
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
AppId={{B1066CB0-F30D-43C5-89BA-D7D8B017D164}
AppName=Tarn
AppVersion={#AppVersion}
AppVerName=Tarn {#AppVersion}
AppPublisher=Quavon UG (haftungsbeschränkt)
AppPublisherURL=https://github.com/Quavon-dev/tarn-browser
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Tarn
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Tarn-v{#AppVersion}-windows-x64-setup
SetupIconFile=tarn.ico
UninstallDisplayIcon={app}\tarn.ico
UninstallDisplayName=Tarn
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=force
RestartApplications=no
SetupMutex=Tarn.Setup
#ifdef Sign
SignTool=tarn
SignedUninstaller=yes
#endif

[InstallDelete]
; A new Chromium build drops files of the old one; none may stay behind next to tarn.exe.
; *.exe and *.dat stay: they would take the uninstaller and its log.
Type: files; Name: "{app}\*.dll"
Type: files; Name: "{app}\*.pak"
Type: files; Name: "{app}\*.bin"
Type: files; Name: "{app}\*.json"
Type: filesandordirs; Name: "{app}\locales"

[UninstallDelete]
; Files Tarn or Chromium wrote next to tarn.exe (a log) would keep the folder from going.
Type: filesandordirs; Name: "{app}"

[Files]
Source: "{#PackageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "tarn.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Tarn"; Filename: "{app}\tarn.exe"; IconFilename: "{app}\tarn.ico"

[Run]
Filename: "{app}\tarn.exe"; Description: "Open Tarn"; Flags: nowait postinstall skipifsilent
Filename: "{app}\tarn.exe"; Flags: nowait; Check: IsTarnUpdate

[Code]
var
  DeleteData: Boolean;

function IsTarnUpdate: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/TARNUPDATE') = 0 then
      Result := True;
end;

{ Restart Manager only sees processes holding files Setup replaces; this also finds a Tarn still shutting down. }
function TarnRunning(const Dir: String): Boolean;
var
  Path: String;
  Locator, Service, Found: Variant;
begin
  Result := False;
  Path := AddBackslash(Dir) + 'tarn.exe';
  StringChangeEx(Path, '\', '\\', True);
  StringChangeEx(Path, '''', '\''', True);
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Found := Service.ExecQuery('SELECT ProcessId FROM Win32_Process WHERE ExecutablePath = ''' + Path + '''');
    Result := Found.Count > 0;
  except
    Log('Could not list processes: ' + GetExceptionMessage);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Waited: Integer;
begin
  { [InstallDelete] clears old runtime files by wildcard, so only Tarn's own folder may be used. }
  if CompareText(RemoveBackslashUnlessRoot(WizardDirValue), ExpandConstant('{localappdata}\Programs\Tarn')) <> 0 then
  begin
    Result := 'Tarn installs only into ' + ExpandConstant('{localappdata}\Programs\Tarn') + '.';
    exit;
  end;
  Result := '';
  Waited := 0;
  while TarnRunning(WizardDirValue) and (Waited < 30) do
  begin
    Sleep(1000);
    Waited := Waited + 1;
  end;
end;

function InitializeUninstall: Boolean;
begin
  Result := not TarnRunning(ExpandConstant('{app}'));
  if not Result then
    SuppressibleMsgBox('Quit Tarn, then uninstall it again.', mbError, MB_OK, IDOK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  Data := ExpandConstant('{localappdata}\Tarn');
  if (CurUninstallStep = usUninstall) and DirExists(Data) then
    DeleteData := SuppressibleMsgBox('Also delete your Tarn data (profiles, history, bookmarks, passwords, cookies and site data) in ' + Data + '?' + #13#10#13#10 + 'Choose No to keep it for a later install.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES;
  if (CurUninstallStep = usPostUninstall) and DeleteData then
    DelTree(Data, True, True, True);
  { Make Default (crates/engine/src/host_windows.rs) registered Tarn as a browser for this user. }
  if CurUninstallStep = usPostUninstall then begin
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Classes\TarnURL');
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Clients\StartMenuInternet\Tarn');
    RegDeleteValue(HKCU, 'Software\RegisteredApplications', 'Tarn');
  end;
end;
