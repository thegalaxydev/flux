; Inno Setup script for the Flux editor + player.
;
; Built from a staged distribution folder (see `cargo xtask dist`). Invoke via:
;   cargo xtask dist --version 0.1.0 --installer
; or directly:
;   ISCC /DFluxVersion=0.1.0 /DStageDir=<path-to-staged-folder> installer\flux.iss
;
; Produces dist\Flux-<ver>-setup.exe. Installs per-user by default (no admin
; prompt); the user may elevate for an all-users install.

#ifndef FluxVersion
  #define FluxVersion "0.0.0"
#endif
#ifndef StageDir
  ; Default matches the xtask's staging layout when run from the repo root.
  #define StageDir "dist\Flux-" + FluxVersion + "-windows-x64"
#endif

[Setup]
; AppId uniquely identifies Flux for upgrades/uninstall — KEEP THIS CONSTANT
; across all future releases, or upgrades will install side-by-side.
AppId={{A3F1B2C4-5D6E-4F7A-8B9C-0D1E2F3A4B5C}
AppName=Flux
AppVersion={#FluxVersion}
AppPublisher=thegalaxydev
AppPublisherURL=https://github.com/thegalaxydev/flux
AppSupportURL=https://github.com/thegalaxydev/flux/issues
DefaultDirName={autopf}\Flux
DefaultGroupName=Flux
UninstallDisplayName=Flux
UninstallDisplayIcon={app}\flux_editor.exe
OutputDir=dist
OutputBaseFilename=Flux-{#FluxVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Default to a per-user install (no UAC); allow elevating to all-users.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The entire self-contained staged folder: exes, flux_script.dll, std-*.dll,
; plugins\flux_game.dll, license notices, README.
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\Flux Editor"; Filename: "{app}\flux_editor.exe"
Name: "{group}\{cm:UninstallProgram,Flux}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Flux Editor"; Filename: "{app}\flux_editor.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\flux_editor.exe"; Description: "{cm:LaunchProgram,Flux}"; Flags: nowait postinstall skipifsilent

; NOTE: no `.scene.json` file association — Windows associates on the final
; extension (`.json`), so registering it would hijack every JSON file. Open
; scenes from the editor's launcher / File > Open instead.
