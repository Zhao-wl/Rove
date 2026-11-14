; Rove 的 Windows 安装器（issue #76 第 2 节）。Inno Setup 6。
;
; 选型理由见 docs/RELEASE.md「安装器选型」。要点：用户级安装、静默升级、CI 无人值守构建都简单。
;
; 用法（`cargo xtask release` 会自动这样调用）：
;   ISCC.exe /DRoveVersion=0.1.0 /DRoveCommit=abcdef /DStageDir=<暂存目录> /DOutputDir=<输出目录> rove.iss
;
; 约定（逐条对应 issue #76 的验收标准）：
; - **用户级安装**：装到 %LOCALAPPDATA%\Programs\Rove，PrivilegesRequired=lowest，不需要管理员权限；
;   只写 HKCU，**绝不写 HKLM**（下面没有任何 HKLM 项，check-iss.ps1 会逐行检查）。
; - 文件关联与右键菜单都是**可选项且默认不勾选**（Tasks 带 unchecked），关联走
;   HKCU\Software\Classes 的 Rove.<ext> ProgID + OpenWithProgids，**不写 (Default)，不抢默认程序**。
; - 右键菜单「用 Rove 打开」对**文件与目录**都提供；目录项把路径当启动参数传给 rove.exe
;   （UX_SPEC §1.6：只接受启动参数，不提供 shell 命令行工具，所以**不往 PATH 里加东西**）。
; - 卸载清除程序目录、快捷方式与上面所有注册表项；用户数据（%LOCALAPPDATA%\Rove）**默认保留**，
;   卸载时可以勾选一并删除。
; - 静默升级（#149）：/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP-；
;   CloseApplications=yes 让安装器等 rove.exe 退出，RestartApplications=no 由更新端决定是否重启。
; - 安装器**不联网**：不捆绑、不下载 WebView2 运行时，也不下载 C# 索引组件
;   （组件包在 packaging\bundled\ 里时才会出现「安装 C# 索引组件」这个可选项）。

#ifndef RoveVersion
  #define RoveVersion "0.0.0"
#endif
#ifndef RoveCommit
  #define RoveCommit "unknown"
#endif
#ifndef StageDir
  #define StageDir "..\..\target\release-dist\stage"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\target\release-dist"
#endif
; 可选：C# 索引组件的 zip（#43 的包，顶层目录就是版本号）。给了就多一个可勾选项。
#ifndef ComponentZip
  #define ComponentZip ""
#endif
#if ComponentZip != ""
  #define ComponentZipName ExtractFileName(ComponentZip)
#endif

#define RoveName "Rove"
#define RovePublisher "Rove"
#define RoveUrl "https://github.com/Zhao-wl/Rove"
#define RoveExe "rove.exe"

[Setup]
AppId={{8E6F6E52-1C4A-4F0B-9E2E-5C1A9E7D4B10}
AppName={#RoveName}
AppVersion={#RoveVersion}
AppVerName={#RoveName} {#RoveVersion}
VersionInfoVersion={#RoveVersion}
AppPublisher={#RovePublisher}
AppPublisherURL={#RoveUrl}
AppSupportURL={#RoveUrl}/issues
AppUpdatesURL={#RoveUrl}/releases
; 用户级安装：不需要管理员权限，不写 HKLM（VS Code 的 User Installer 也是这样）。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={localappdata}\Programs\{#RoveName}
DefaultGroupName={#RoveName}
DisableProgramGroupPage=yes
; 卸载信息写进 HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall。
UninstallDisplayName={#RoveName} {#RoveVersion}
UninstallDisplayIcon={app}\{#RoveExe}
OutputDir={#OutputDir}
OutputBaseFilename=Rove-{#RoveVersion}-win-x64-setup
SetupIconFile=rove.ico
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; 升级与静默安装（#149 的无人值守路径走的就是这里）。
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
; 不做 Authenticode 签名（D11）：首次运行会看到 SmartScreen 提示，README 里写了放行方法。
LicenseFile={#StageDir}\LICENSE.txt

[Languages]
; Inno Setup 6 自带的只有英文（Default.isl）；中文 .isl 要另外下载，CI 上装不了，
; 所以向导本身用自带的英文，Rove 自己的选项说明（[Tasks] / [Run]）写中文。
Name: "default"; MessagesFile: "compiler:Default.isl"

[Tasks]
; 全部默认不勾选（照搬 VS Code 安装器的习惯）。
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式:"; Flags: unchecked
Name: "fileassoc"; Description: "把 Rove 添加到 .cs、.json、.yaml、.xml、.md、.log 等类型的「打开方式」（不设为默认程序）"; GroupDescription: "文件关联与右键菜单:"; Flags: unchecked
Name: "contextmenu"; Description: "在资源管理器的文件右键菜单中显示「用 Rove 打开」"; GroupDescription: "文件关联与右键菜单:"; Flags: unchecked
Name: "contextmenudir"; Description: "在资源管理器的目录右键菜单中显示「用 Rove 打开」"; GroupDescription: "文件关联与右键菜单:"; Flags: unchecked
#if ComponentZip != ""
Name: "indexer"; Description: "安装 C# 索引组件（可选，离线安装，约 90 MB）"; GroupDescription: "可选组件:"; Flags: unchecked
#endif

[Files]
; 暂存目录由 cargo xtask release 按 packaging/package-contents.toml 组装。
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
#if ComponentZip != ""
; 组件包随安装器携带，**本地解压**到 #43 约定的位置；安装器绝不联网下载。
Source: "{#ComponentZip}"; DestDir: "{tmp}"; Flags: deleteafterinstall; Tasks: indexer
#endif

[Icons]
Name: "{autoprograms}\{#RoveName}"; Filename: "{app}\{#RoveExe}"
Name: "{autodesktop}\{#RoveName}"; Filename: "{app}\{#RoveExe}"; Tasks: desktopicon

[Registry]
; ---- 文件关联（可选）：HKCU\Software\Classes 下的 ProgID + OpenWithProgids，不写 (Default)，
;      所以不会抢默认程序，只是出现在「打开方式」里。卸载时 uninsdeletekey 全部删掉。
Root: HKCU; Subkey: "Software\Classes\Rove.SourceFile"; ValueType: string; ValueName: ""; ValueData: "Rove 源文件"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\Rove.SourceFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#RoveExe},0"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\Rove.SourceFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#RoveExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.cs\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.json\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.jsonc\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.yaml\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.yml\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.xml\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.log\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.shader\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.hlsl\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.lua\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\.asmdef\OpenWithProgids"; ValueType: string; ValueName: "Rove.SourceFile"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc

; ---- 右键菜单「用 Rove 打开」（可选）：文件与目录各一套，卸载时整键删除。
Root: HKCU; Subkey: "Software\Classes\*\shell\Rove"; ValueType: string; ValueName: ""; ValueData: "用 Rove 打开"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\Rove"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#RoveExe}"""; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\Rove\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#RoveExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: contextmenu
; 目录本身：把目录路径作为启动参数 → 以它为工作区打开。
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Rove"; ValueType: string; ValueName: ""; ValueData: "用 Rove 打开"; Flags: uninsdeletekey; Tasks: contextmenudir
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Rove"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#RoveExe}"""; Tasks: contextmenudir
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Rove\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#RoveExe}"" ""%V"""; Flags: uninsdeletekey; Tasks: contextmenudir
; 在目录空白处右键（background）：%V 是当前目录。
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Rove"; ValueType: string; ValueName: ""; ValueData: "用 Rove 打开"; Flags: uninsdeletekey; Tasks: contextmenudir
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Rove"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#RoveExe}"""; Tasks: contextmenudir
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Rove\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#RoveExe}"" ""%V"""; Flags: uninsdeletekey; Tasks: contextmenudir

#if ComponentZip != ""
[Dirs]
; 组件的安装位置（#43 的发现规则）；用户级目录，不需要管理员权限。
Name: "{localappdata}\Rove\components\indexer-cs"; Tasks: indexer
#endif

[Run]
#if ComponentZip != ""
; C# 索引组件：把随包的 zip 解压到 %LOCALAPPDATA%\Rove\components\indexer-cs\
; （zip 的顶层目录就是版本号，正好满足 #43 的发现规则）。用系统自带的 tar，不联网。
Filename: "{sys}\tar.exe"; Parameters: "-xf ""{tmp}\{#ComponentZipName}"" -C ""{localappdata}\Rove\components\indexer-cs"""; StatusMsg: "正在安装 C# 索引组件…"; Flags: runhidden; Tasks: indexer
#endif
Filename: "{app}\{#RoveExe}"; Description: "启动 Rove"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 程序目录里运行期产生的东西（日志、缓存）一并删掉，留下空目录也删掉。
Type: filesandordirs; Name: "{app}\components"
Type: dirifempty; Name: "{app}"

[Code]
var
  RemoveUserDataCheckBox: TNewCheckBox;

// 卸载进度页上加一个「同时删除用户数据」的勾选项，**默认不勾**
//（issue #76：用户数据默认保留）。静默卸载时不显示，按默认保留。
procedure InitializeUninstallProgressForm();
begin
  if UninstallSilent() then
    Exit;
  RemoveUserDataCheckBox := TNewCheckBox.Create(UninstallProgressForm);
  RemoveUserDataCheckBox.Parent := UninstallProgressForm.InnerPage;
  RemoveUserDataCheckBox.Left := UninstallProgressForm.StatusLabel.Left;
  RemoveUserDataCheckBox.Top := UninstallProgressForm.StatusLabel.Top +
    UninstallProgressForm.StatusLabel.Height + ScaleY(16);
  RemoveUserDataCheckBox.Width := UninstallProgressForm.InnerPage.ClientWidth -
    RemoveUserDataCheckBox.Left * 2;
  RemoveUserDataCheckBox.Height := ScaleY(34);
  RemoveUserDataCheckBox.WordWrap := True;
  RemoveUserDataCheckBox.Caption :=
    '同时删除用户数据（%LOCALAPPDATA%\Rove：索引缓存、设置备份、工作区状态）';
  RemoveUserDataCheckBox.Checked := False;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and (RemoveUserDataCheckBox <> nil) and
     RemoveUserDataCheckBox.Checked then
    DelTree(ExpandConstant('{localappdata}\Rove'), True, True, True);
end;
