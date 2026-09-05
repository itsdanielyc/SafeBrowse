; Per-user installation and removal share the application's maintenance exclusion.
; The build script supplies reviewed binaries and a verified Microsoft bootstrapper.
#if Ver < EncodeVer(7, 1, 0)
  #error Inno Setup 7.1.0 or newer is required.
#endif
#ifndef RootDirectory
  #error RootDirectory must identify the SafeBrowse source directory.
#endif
#ifndef AppBinaryDirectory
  #error AppBinaryDirectory must identify the reviewed release binaries.
#endif
#ifndef RuntimeBootstrapperPath
  #error RuntimeBootstrapperPath must identify the verified Microsoft bootstrapper.
#endif
#ifndef OutputDirectory
  #error OutputDirectory must identify the installer output directory.
#endif
#ifndef AppVersion
  #error AppVersion must contain the package version.
#endif
#ifndef MinimumRuntimeVersion
  #error MinimumRuntimeVersion must contain the application's runtime support floor.
#endif

#define SafeBrowseAppId "fd54b727-cea2-44b9-8c39-25c704348aea"

[Setup]
AppId={{{#SafeBrowseAppId}}
AppName=SafeBrowse
AppVersion={#AppVersion}
AppPublisher=SafeBrowse Contributors
AppPublisherURL=https://github.com/itsdanielyc/SafeBrowse
AppSupportURL=https://github.com/itsdanielyc/SafeBrowse/issues
AppUpdatesURL=https://github.com/itsdanielyc/SafeBrowse/releases
AppCopyright=Copyright (c) 2026 SafeBrowse Contributors
VersionInfoVersion={#AppVersion}
VersionInfoDescription=SafeBrowse Setup
DefaultDirName={localappdata}\Programs\SafeBrowse
DisableDirPage=yes
UsePreviousAppDir=no
AppendDefaultDirName=no
DisableProgramGroupPage=yes
DefaultGroupName=SafeBrowse
PrivilegesRequired=lowest
SetupArchitecture=x64
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0.19041
AppMutex=Local\SafeBrowse_Session_Mutex
SetupMutex=Local\SafeBrowse_Setup_Mutex
CloseApplications=no
RestartApplications=no
AlwaysRestart=no
RestartIfNeededByRun=no
DisableWelcomePage=no
WizardStyle=modern dynamic
LicenseFile={#RootDirectory}\LICENSE
SetupIconFile={#RootDirectory}\assets\branding\safebrowse.ico
UninstallDisplayIcon={app}\safebrowse.exe
UninstallDisplayName=SafeBrowse
Compression=lzma2
SolidCompression=yes
OutputDir={#OutputDirectory}
OutputBaseFilename=SafeBrowse-Setup-{#AppVersion}-x64
SignedUninstaller=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
; Put prerequisite inputs first so early extraction does not unpack the application.
Source: "{#AppBinaryDirectory}\safebrowse-maintenance.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RuntimeBootstrapperPath}"; DestName: "MicrosoftEdgeWebview2Setup.exe"; Flags: dontcopy
Source: "{#AppBinaryDirectory}\safebrowse.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RootDirectory}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RootDirectory}\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\SafeBrowse"; Filename: "{app}\safebrowse.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\SafeBrowse"; Filename: "{app}\safebrowse.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Code]
const
  ApplicationMutexName = 'Local\SafeBrowse_Session_Mutex';
  MaintenanceMutexName = 'Local\SafeBrowse_Maintenance_Mutex';
  InstalledVersionKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{{#SafeBrowseAppId}}_is1';
  InstallerVersion = '{#AppVersion}';
  SupportedRuntimeVersion = '{#MinimumRuntimeVersion}';
  MaintenanceExecutableName = 'safebrowse-maintenance.exe';
  BootstrapperExecutableName = 'MicrosoftEdgeWebview2Setup.exe';
  RuntimeInstallationNeeded = 10;
  WindowsRestartRequired = 3010;
  WindowsAlreadyExists = 183;
  MaximumDiagnosticCharacters = 2000;
  ElevatedOperationMessage = 'Open SafeBrowse setup or removal without "Run as administrator", using the Windows account that installed it. No application files or data were changed.';
  ApplicationRunningMessage = 'Close SafeBrowse before installing or removing it. SafeBrowse will not close an active browsing session automatically.';

var
  MaintenanceMutexHandle: HANDLE;
  MaintenanceExecutable: String;

function WindowsCreateMutex(SecurityAttributes: ULONG_PTR; InitialOwner: BOOL; Name: String): HANDLE;
  external 'CreateMutexW@kernel32.dll stdcall';
function WindowsGetLastError: DWORD;
  external 'GetLastError@kernel32.dll stdcall';
procedure WindowsSetLastError(ErrorCode: DWORD);
  external 'SetLastError@kernel32.dll stdcall';
function WindowsCloseHandle(ObjectHandle: HANDLE): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

procedure ShowOperationError(const MessageText: String);
begin
  Log(MessageText);
  SuppressibleMsgBox(MessageText, mbError, MB_OK, IDOK);
end;

{ Hold exclusion through all file changes, not merely the prerequisite check. }
function AcquireMaintenanceExclusion: Boolean;
var
  ErrorCode: DWORD;
begin
  Result := False;
  if IsAdmin then begin
    ShowOperationError(ElevatedOperationMessage);
    Exit;
  end;
  WindowsSetLastError(0);
  MaintenanceMutexHandle := WindowsCreateMutex(0, False, MaintenanceMutexName);
  ErrorCode := WindowsGetLastError;
  if MaintenanceMutexHandle = 0 then begin
    ShowOperationError('Cannot reserve SafeBrowse for installation or removal. Close other SafeBrowse setup windows and retry. (' + SysErrorMessage(ErrorCode) + ')');
    Exit;
  end;
  if ErrorCode = WindowsAlreadyExists then begin
    WindowsCloseHandle(MaintenanceMutexHandle);
    MaintenanceMutexHandle := 0;
    ShowOperationError('Another SafeBrowse installation or removal is active. Wait for it to finish, then retry.');
    Exit;
  end;
  Result := True;
end;

procedure ReleaseMaintenanceExclusion;
begin
  if MaintenanceMutexHandle <> 0 then begin
    WindowsCloseHandle(MaintenanceMutexHandle);
    MaintenanceMutexHandle := 0;
  end;
end;

{ Only released three-part numeric versions can participate in downgrade checks. }
function ParsePackageVersion(const VersionText: String; var PackedVersion: Int64): Boolean;
var
  CharacterIndex, SeparatorCount, ComponentDigits: Integer;
begin
  Result := False;
  SeparatorCount := 0;
  ComponentDigits := 0;
  if VersionText = '' then Exit;
  for CharacterIndex := 1 to Length(VersionText) do begin
    if VersionText[CharacterIndex] = '.' then begin
      if ComponentDigits = 0 then Exit;
      SeparatorCount := SeparatorCount + 1;
      ComponentDigits := 0;
    end else begin
      if (VersionText[CharacterIndex] < '0') or (VersionText[CharacterIndex] > '9') then Exit;
      ComponentDigits := ComponentDigits + 1;
    end;
  end;
  if (SeparatorCount <> 2) or (ComponentDigits = 0) then Exit;
  Result := StrToVersion(VersionText, PackedVersion);
end;

function ValidateInstalledVersion: String;
var
  InstalledVersion: String;
  InstalledPackedVersion, InstallerPackedVersion: Int64;
begin
  Result := '';
  if not ParsePackageVersion(InstallerVersion, InstallerPackedVersion) then begin
    Result := 'This SafeBrowse installer has invalid version metadata. Obtain a new installer.';
    Exit;
  end;
  if not RegKeyExists(HKCU64, InstalledVersionKey) then Exit;
  if not RegQueryStringValue(HKCU64, InstalledVersionKey, 'DisplayVersion', InstalledVersion) then begin
    Result := 'The installed SafeBrowse version could not be read. Remove the existing installation through Windows Installed apps before installing again.';
    Exit;
  end;
  if not ParsePackageVersion(InstalledVersion, InstalledPackedVersion) then begin
    Result := 'The installed SafeBrowse version is not recognised. Remove the existing installation through Windows Installed apps before installing again.';
    Exit;
  end;
  if ComparePackedVersion(InstalledPackedVersion, InstallerPackedVersion) > 0 then
    Result := 'SafeBrowse ' + InstalledVersion + ' is already installed. This installer contains the older version ' + InstallerVersion + '. Use the same version or a newer installer.';
end;

function InitializeSetup: Boolean;
var
  VersionError: String;
begin
  Result := AcquireMaintenanceExclusion;
  if not Result then Exit;
  VersionError := ValidateInstalledVersion;
  if VersionError <> '' then begin
    ShowOperationError(VersionError);
    Result := False;
  end;
end;

procedure DeinitializeSetup;
begin
  ReleaseMaintenanceExclusion;
end;

function InitializeUninstall: Boolean;
begin
  Result := AcquireMaintenanceExclusion;
end;

procedure DeinitializeUninstall;
begin
  ReleaseMaintenanceExclusion;
end;

{ Time O(L), space O(C): output lines L, bounded visible characters C. }
function DescribeCapturedOutput(const Lines: TArrayOfString): String;
var
  LineIndex: Integer;
begin
  Result := '';
  for LineIndex := 0 to GetArrayLength(Lines) - 1 do begin
    if Result <> '' then Result := Result + #13#10;
    Result := Result + Copy(Lines[LineIndex], 1, MaximumDiagnosticCharacters + 1);
    if Length(Result) > MaximumDiagnosticCharacters then begin
      Result := Copy(Result, 1, MaximumDiagnosticCharacters) + #13#10 + 'Further details were written to the installation or removal log, when logging is enabled.';
      Exit;
    end;
  end;
end;

{ Keep stderr separate; a capture failure must never look like successful cleanup. }
function RunMaintenance(const Arguments: String; var ExitCode: Integer; var Details: String): Boolean;
var
  CapturedOutput: TExecOutput;
  Launched: Boolean;
  LineIndex: Integer;
begin
  Result := False;
  ExitCode := -1;
  Details := '';
  try
    Launched := ExecAndCaptureOutput(MaintenanceExecutable, Arguments, ExtractFileDir(MaintenanceExecutable), SW_SHOWNORMAL, ewWaitUntilTerminated, ExitCode, CapturedOutput);
    if not Launched then begin
      Details := 'Cannot start SafeBrowse maintenance: ' + SysErrorMessage(ExitCode);
      Exit;
    end;
    for LineIndex := 0 to GetArrayLength(CapturedOutput.StdErr) - 1 do Log(CapturedOutput.StdErr[LineIndex]);
    for LineIndex := 0 to GetArrayLength(CapturedOutput.StdOut) - 1 do Log(CapturedOutput.StdOut[LineIndex]);
    if CapturedOutput.Error then begin
      Details := 'SafeBrowse maintenance output could not be read completely. Retry the operation; removal has not continued.';
      Exit;
    end;
    Details := DescribeCapturedOutput(CapturedOutput.StdErr);
    if Details = '' then Details := DescribeCapturedOutput(CapturedOutput.StdOut);
    if (Details = '') and (ExitCode <> 0) then
      Details := 'SafeBrowse maintenance ended with exit code ' + IntToStr(ExitCode) + '.';
    Result := True;
  except
    Details := 'Cannot run SafeBrowse maintenance: ' + GetExceptionMessage;
  end;
end;

function EnsureSupportedRuntime: String;
var
  ExitCode: Integer;
  Details: String;
begin
  Result := '';
  if not RunMaintenance('check-runtime', ExitCode, Details) then begin
    Result := Details;
    Exit;
  end;
  if ExitCode = 0 then Exit;
  if ExitCode <> RuntimeInstallationNeeded then begin
    Result := Details;
    Exit;
  end;
  WizardForm.PreparingMemo.Lines.Text := 'Installing Microsoft Edge WebView2. Internet access is required. Setup will continue when Microsoft''s installer finishes.';
  WizardForm.PreparingMemo.Visible := True;
  try
    try
      ExtractTemporaryFile(BootstrapperExecutableName);
      if not Exec(ExpandConstant('{tmp}\') + BootstrapperExecutableName, '/silent /install', ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then begin
        Result := 'Cannot start Microsoft WebView2 installation. Check your connection and retry. (' + SysErrorMessage(ExitCode) + ')';
        Exit;
      end;
    except
      Result := 'Cannot prepare Microsoft WebView2 installation: ' + GetExceptionMessage;
      Exit;
    end;
    if (ExitCode <> 0) and (ExitCode <> WindowsRestartRequired) then begin
      Result := 'Microsoft WebView2 installation did not finish (exit code ' + IntToStr(ExitCode) + '). Connect to the internet and retry. If your computer is managed, ask your administrator about WebView2 installation.';
      Exit;
    end;
    if not RunMaintenance('check-runtime', ExitCode, Details) then begin
      Result := Details;
      Exit;
    end;
    if ExitCode <> 0 then
      Result := 'A supported Microsoft WebView2 Runtime is still unavailable. Restart Windows if Microsoft requested it, then run this installer again.' + #13#10#13#10 + Details;
  finally
    WizardForm.PreparingMemo.Visible := False;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  NeedsRestart := False;
  if not SameText(RemoveBackslashUnlessRoot(ExpandConstant('{app}')), RemoveBackslashUnlessRoot(ExpandConstant('{localappdata}\Programs\SafeBrowse'))) then begin
    Result := 'SafeBrowse must be installed in its standard folder for this Windows account. Run the installer without a custom destination.';
    Exit;
  end;
  if CheckForMutexes(ApplicationMutexName) then begin
    Result := ApplicationRunningMessage;
    Exit;
  end;
  Result := ValidateInstalledVersion;
  if Result <> '' then Exit;
  try
    ExtractTemporaryFile(MaintenanceExecutableName);
    MaintenanceExecutable := ExpandConstant('{tmp}\') + MaintenanceExecutableName;
  except
    Result := 'Cannot prepare SafeBrowse installation checks: ' + GetExceptionMessage;
    Exit;
  end;
  Result := EnsureSupportedRuntime;
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo, MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := MemoDirInfo + NewLine + NewLine;
  if MemoTasksInfo <> '' then Result := Result + MemoTasksInfo + NewLine + NewLine;
  Result := Result + 'Microsoft Edge WebView2:' + NewLine + Space + 'Setup checks for runtime ' + SupportedRuntimeVersion + ' or newer. Internet access is needed only if Microsoft WebView2 must be installed or updated. This shared component is kept when SafeBrowse is removed.';
end;

{ This event runs after confirmation and before installed files are deleted. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  RemoveUserData: Boolean;
  Choice, ExitCode: Integer;
  Arguments, Details: String;
begin
  if CurUninstallStep <> usUninstall then Exit;
  RemoveUserData := False;
  if not UninstallSilent then begin
    Choice := SuppressibleMsgBox('Also remove SafeBrowse settings and browsing data?' + #13#10#13#10 + 'Yes removes bookmarks, preferences, saved site permissions and SafeBrowse''s persistent browser profile.' + #13#10#13#10 + 'No keeps these for a future installation. Downloaded files and the shared Microsoft WebView2 Runtime are always kept.', mbConfirmation, MB_YESNOCANCEL or MB_DEFBUTTON2, IDNO);
    if Choice = IDCANCEL then Abort;
    RemoveUserData := Choice = IDYES;
  end;
  if CheckForMutexes(ApplicationMutexName) then begin
    ShowOperationError(ApplicationRunningMessage);
    Abort;
  end;
  MaintenanceExecutable := ExpandConstant('{app}\') + MaintenanceExecutableName;
  Arguments := 'cleanup';
  if RemoveUserData then Arguments := Arguments + ' --remove-user-data';
  if not RunMaintenance(Arguments, ExitCode, Details) then begin
    ShowOperationError('SafeBrowse removal stopped before deleting application files.' + #13#10#13#10 + Details);
    Abort;
  end;
  if ExitCode <> 0 then begin
    ShowOperationError('SafeBrowse removal stopped before deleting application files. Some data may already have been removed; retry removal after resolving the reported problem.' + #13#10#13#10 + Details);
    Abort;
  end;
end;
