; Inno Setup script for the Device Studio Windows installer.
; Built by tools/release/package-windows.ps1, which passes AppVersion, SourceDir (the staged
; app folder) and OutputDir.

#ifndef AppVersion
  #error AppVersion is not defined
#endif

[Setup]
; A fixed id: later versions upgrade this install instead of adding a second one.
AppId={{6C1C7F5E-3B47-4E0B-9C1F-5D2B8F0A4E61}
AppName=Device Studio
AppVersion={#AppVersion}
AppVerName=Device Studio {#AppVersion}
DefaultDirName={autopf}\Device Studio
DefaultGroupName=Device Studio
UninstallDisplayIcon={app}\device-studio.exe
OutputDir={#OutputDir}
OutputBaseFilename=DeviceStudio-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Per-user install by default (no admin prompt); the user can choose all users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
#ifexist SourceDir + "\LICENSE"
LicenseFile={#SourceDir}\LICENSE
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\Device Studio"; Filename: "{app}\device-studio.exe"
Name: "{group}\Uninstall Device Studio"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Device Studio"; Filename: "{app}\device-studio.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\device-studio.exe"; Description: "{cm:LaunchProgram,Device Studio}"; Flags: nowait postinstall skipifsilent
