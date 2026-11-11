; Installeur Inno Setup de FaceSort.
; Ne pas compiler directement : utiliser scripts\package.ps1, qui construit
; dist\FaceSort-<version>-portable\ puis appelle :
;   iscc /DMyAppVersion=<version> /DSourceDir=<dossier portable> installer\FaceSort.iss
; Le setup est créé dans installer\Output\FaceSort-Setup-<version>.exe
#define MyAppExe "FaceSort.exe"
#define MyAppName "FaceSort"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\FaceSort-portable"
#endif
#define MyAppURL "https://github.com/Les-Doubles-A/FaceSort"

[Setup]
AppId={{6400BE40-A3CB-4E82-AB78-7AB28FB4D91E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=Les-Doubles-A
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
VersionInfoVersion={#MyAppVersion}
VersionInfoDescription=Installeur de {#MyAppName}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=Output
OutputBaseFilename={#MyAppName}-Setup-{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
SetupIconFile=..\FaceSort.ico
UninstallDisplayIcon={app}\{#MyAppExe}
MinVersion=10.0

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Exécutables, DirectML, runtime Visual C++ (déploiement local), modèles, documents
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

; La base %LOCALAPPDATA%\FaceSort\facesort.db (prénoms, visages) n'est pas
; supprimée à la désinstallation : l'utilisateur la supprime s'il le souhaite.
