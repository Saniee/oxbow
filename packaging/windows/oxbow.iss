; Oxbow's Windows installer (Inno Setup 6 or later). Built by build.ps1, which passes Version, Source and OutputDir.
; Installs for the current user by default, without admin rights; the first page offers all users instead.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
; Never change AppId: it is how an update finds the installed copy.
AppId={{42359147-68AE-4F0B-A9AA-FA6FACAB44F6}
AppName=Oxbow
AppVersion={#Version}
AppVerName=Oxbow {#Version}
AppPublisher=Saniee
AppPublisherURL=https://github.com/Saniee/oxbow
AppSupportURL=https://github.com/Saniee/oxbow/issues
DefaultDirName={autopf}\Oxbow
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=oxbow-{#Version}-windows-x64-setup
SetupIconFile=..\..\crates\app\assets\icon.ico
UninstallDisplayIcon={app}\oxbow.exe
UninstallDisplayName=Oxbow
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; An open Oxbow is closed for the update and started again after it.
CloseApplications=yes
RestartApplications=yes
; Tells open terminals' parent (Explorer) that PATH changed, so new terminals find oxbow.
ChangesEnvironment=yes

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: addtopath; Description: "Add Oxbow to PATH, to open a folder from a terminal with ""oxbow <folder>"""; GroupDescription: "Other:"

[Registry]
; The current user's PATH, or everyone's when installing for all users.
Root: HKA; Subkey: "{code:EnvironmentKey}"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Files]
Source: "{#Source}\oxbow.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Oxbow"; Filename: "{app}\oxbow.exe"
Name: "{autodesktop}\Oxbow"; Filename: "{app}\oxbow.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\oxbow.exe"; Description: "{cm:LaunchProgram,Oxbow}"; Flags: nowait postinstall skipifsilent
; Oxbow's own updates run Setup silently with /RELAUNCH=yes (and /OPEN=<folder>) to be started again afterwards,
; as the user and not as an administrator.
Filename: "{app}\oxbow.exe"; Parameters: "{code:ReopenArgs}"; Flags: nowait runasoriginaluser; Check: Relaunch

; Settings and the recent projects list (%APPDATA%\Oxbow) are left in place on uninstall.

[Code]
// Oxbow runs the git that is installed, so say so when there is none. It doesn't stop the install.
function InitializeSetup(): Boolean;
begin
  if (FileSearch('git.exe', GetEnv('PATH')) = '') and not WizardSilent() then
    MsgBox('Oxbow uses Git, and Git wasn''t found on this computer.' + #13#10#13#10 +
      'Install it from https://git-scm.com before you start Oxbow.', mbInformation, MB_OK);
  Result := True;
end;

function Relaunch(): Boolean;
begin
  Result := CompareText(ExpandConstant('{param:relaunch|no}'), 'yes') = 0;
end;

function ReopenArgs(Param: String): String;
var
  Folder: String;
begin
  Folder := ExpandConstant('{param:open|}');
  if Folder = '' then
    Result := ''
  else
    Result := AddQuotes(Folder);
end;

function EnvironmentKey(Param: String): String;
begin
  if IsAdminInstallMode() then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

function EnvironmentRoot(): Integer;
begin
  if IsAdminInstallMode() then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function SameDir(A, B: String): Boolean;
begin
  Result := CompareText(RemoveBackslashUnlessRoot(Trim(A)), RemoveBackslashUnlessRoot(Trim(B))) = 0;
end;

// PATH without `Dir`, entry by entry.
function PathWithout(Paths, Dir: String): String;
var
  P: Integer;
  Part: String;
begin
  Result := '';
  while Paths <> '' do
  begin
    P := Pos(';', Paths);
    if P = 0 then
    begin
      Part := Paths;
      Paths := '';
    end
    else
    begin
      Part := Copy(Paths, 1, P - 1);
      Delete(Paths, 1, P);
    end;
    if (Part <> '') and not SameDir(Part, Dir) then
    begin
      if Result <> '' then
        Result := Result + ';';
      Result := Result + Part;
    end;
  end;
end;

function NeedsAddPath(Dir: String): Boolean;
var
  Paths: String;
begin
  if not RegQueryStringValue(EnvironmentRoot(), EnvironmentKey(''), 'Path', Paths) then
    Result := True
  else
    Result := PathWithout(Paths, Dir) = PathWithout(Paths, '');
end;

// Uninstalling takes Oxbow's folder out of PATH again, if it was put there.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Paths, Rest: String;
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  if not RegQueryStringValue(EnvironmentRoot(), EnvironmentKey(''), 'Path', Paths) then
    exit;
  Rest := PathWithout(Paths, ExpandConstant('{app}'));
  if Rest <> PathWithout(Paths, '') then
    RegWriteExpandStringValue(EnvironmentRoot(), EnvironmentKey(''), 'Path', Rest);
end;
