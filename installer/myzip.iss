; MyZIP 설치 스크립트 (Inno Setup 6)
;
; 빌드:  python build.py --installer
;   또는  "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\myzip.iss
;
; 관리자 권한을 요구하지 않는다. MyZIP 이 만드는 레지스트리 항목은 전부
; HKEY_CURRENT_USER 아래라 사용자 권한으로 충분하고, UAC 프롬프트 없이
; 설치가 끝나는 편이 사용자에게 낫기 때문이다.

#define AppName        "MyZIP"
#define AppVersion     "1.0.0"
#define AppPublisher   "MyZIP"
#define AppExeName     "MyZIP.exe"
#define SourceDir      "..\dist\MyZIP"

[Setup]
AppId={{8F3A5C21-7D4E-4B9A-9C12-5E7A3D8B4F60}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
VersionInfoVersion={#AppVersion}

; 관리자 권한 없이 사용자 폴더에 설치한다.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
AllowNoIcons=yes

OutputDir=output
OutputBaseFilename=MyZIP-{#AppVersion}-setup
SetupIconFile=..\resources\icons\app.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName} {#AppVersion}

Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible

; 실행 중이면 알려 주고 닫게 한다
CloseApplications=yes
CloseApplicationsFilter=*.exe,*.dll
RestartApplications=no

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
korean.AssocTask=압축 파일을 MyZIP 으로 열기 (.zip .tar .tgz .rar .7z)
korean.MenuTask=탐색기 마우스 오른쪽 메뉴에 MyZIP 넣기
korean.DesktopTask=바탕 화면에 바로 가기 만들기
korean.ShellGroup=Windows 통합
korean.LaunchApp=MyZIP 실행하기
korean.Win11Note=Windows 11 에서는 우클릭 후 '추가 옵션 표시' 안에 나타납니다.

english.AssocTask=Open archives with MyZIP (.zip .tar .tgz .rar .7z)
english.MenuTask=Add MyZIP to the Explorer right-click menu
english.DesktopTask=Create a desktop shortcut
english.ShellGroup=Windows integration
english.LaunchApp=Launch MyZIP
english.Win11Note=On Windows 11 this appears under "Show more options".

[Tasks]
Name: "shellmenu"; Description: "{cm:MenuTask}"; \
    GroupDescription: "{cm:ShellGroup}"
Name: "fileassoc"; Description: "{cm:AssocTask}"; \
    GroupDescription: "{cm:ShellGroup}"
Name: "desktopicon"; Description: "{cm:DesktopTask}"; \
    Flags: unchecked

[Files]
Source: "{#SourceDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\*"; DestDir: "{app}"; \
    Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; \
    Tasks: desktopicon

[Run]
; 확장자 연결과 컨텍스트 메뉴를 함께 등록
Filename: "{app}\{#AppExeName}"; Parameters: "--register"; \
    Flags: runhidden waituntilterminated; Tasks: fileassoc

; 컨텍스트 메뉴만 등록 (확장자 연결을 고르지 않은 경우)
Filename: "{app}\{#AppExeName}"; Parameters: "--register-menu"; \
    Flags: runhidden waituntilterminated; Tasks: shellmenu and not fileassoc

Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchApp}"; \
    Flags: nowait postinstall skipifsilent

[UninstallRun]
; 제거 전에 우리가 만든 레지스트리 항목을 스스로 치우게 한다.
Filename: "{app}\{#AppExeName}"; Parameters: "--unregister"; \
    Flags: runhidden waituntilterminated; RunOnceId: "UnregisterShell"

[UninstallDelete]
; 설정은 레지스트리(HKCU\Software\MyZIP)에 있고 --unregister 가 지운다.
Type: filesandordirs; Name: "{app}"

[Code]
{ 설치가 끝난 뒤 Windows 11 사용자에게 컨텍스트 메뉴 위치를 알려 준다. }
function IsWindows11: Boolean;
var
  Version: TWindowsVersion;
begin
  GetWindowsVersionEx(Version);
  Result := (Version.Major > 10) or
            ((Version.Major = 10) and (Version.Build >= 22000));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  { 무인 설치(/SILENT, /VERYSILENT)에서는 안내창을 띄우지 않는다.
    자동 배포 스크립트가 여기서 멈추면 곤란하다. }
  if (CurStep = ssPostInstall) and IsWindows11
     and WizardIsTaskSelected('shellmenu')
     and (not WizardSilent) then
    MsgBox(ExpandConstant('{cm:Win11Note}'), mbInformation, MB_OK);
end;
