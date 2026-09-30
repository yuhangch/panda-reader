; Panda Reader Windows installer (Inno Setup 6).
; Compiled by bundle-windows.ps1 with /D defines:
;   AppVersion, VersionInfoVersion, StageDir, OutputDir, OutputName

#ifndef AppVersion
  #error Missing /DAppVersion
#endif
#ifndef VersionInfoVersion
  #error Missing /DVersionInfoVersion
#endif

[Setup]
AppId={{B7E2C4A1-9F38-4D6E-A1C5-8E2F0D47B9C3}
AppName=Panda Reader
AppVersion={#AppVersion}
VersionInfoVersion={#VersionInfoVersion}
AppPublisher=Panda Reader contributors
AppPublisherURL=https://github.com/yuhangch/panda-reader
AppSupportURL=https://github.com/yuhangch/panda-reader/issues
AppUpdatesURL=https://github.com/yuhangch/panda-reader/releases
DefaultDirName={autopf}\Panda Reader
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile={#StageDir}\LICENSE.txt
SetupIconFile=..\..\apps\panda-reader\assets\app-icon.ico
UninstallDisplayIcon={app}\panda-reader.exe
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\panda-reader.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_ASSET_LICENSES\*"; DestDir: "{app}\THIRD_PARTY_ASSET_LICENSES"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Panda Reader"; Filename: "{app}\panda-reader.exe"
Name: "{autodesktop}\Panda Reader"; Filename: "{app}\panda-reader.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\panda-reader.exe"; Description: "{cm:LaunchProgram,Panda Reader}"; Flags: nowait postinstall skipifsilent
