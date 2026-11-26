; Instalador de Surco para Windows (Inno Setup 6).
; Uso: ISCC.exe /DAppVersion=0.1.0 /DSourceExe=C:\ruta\surco.exe /OC:\ruta\dist packaging\windows\surco.iss
;
; Se instala por usuario (no pide permisos de administrador), en
; %LOCALAPPDATA%\Programs\Surco, con acceso directo en el menú Inicio.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\surco.exe"
#endif

[Setup]
AppId={{6B0C7A52-3E0B-4C55-9C7E-5C1F2D8A9B31}
AppName=Surco
AppVersion={#AppVersion}
AppPublisher=mathiux
AppPublisherURL=https://github.com/devMathiuX/Surco
DefaultDirName={autopf}\Surco
DefaultGroupName=Surco
AppCopyright=© 2026 mathiux. Licencia MIT.
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputBaseFilename=Surco-{#AppVersion}-windows-x64-setup
SetupIconFile=..\icons\surco.ico
UninstallDisplayIcon={app}\surco.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "surco.exe"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\THIRD_PARTY_LICENSES.html"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Surco"; Filename: "{app}\surco.exe"
Name: "{autodesktop}\Surco"; Filename: "{app}\surco.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\surco.exe"; Description: "{cm:LaunchProgram,Surco}"; Flags: nowait postinstall skipifsilent
