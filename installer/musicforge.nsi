; ============================================================================
;  MusicForge (MusicForge) v0.1.0 — Windows 安装脚本
;
;  设计约束（与项目 11 条硬约束一致）：
;    · 离线优先 —— 安装器不联网、不下载任何组件
;    · 不上传、不收集数据 —— 无遥测、无统计回传
;    · 不劫持 —— 不注册文件关联、不设开机自启、不改 PATH
;    · 单用户安装（RequestExecutionLevel user）—— 不申请管理员权限
; ============================================================================

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"

; ---------------------------------------------------------------- 基本定义 --
!define APPNAME    "MusicForge"
!define APPDESC    "本地 NCM 转换器（离线 · 不联网 · 不上传）"

; 版本号：默认 0.1.0，可由命令行覆盖
;   makensis /DVERSION=0.2.0 musicforge.nsi
!ifndef VERSION
  !define VERSION  "0.1.1"
!endif

!define PUBLISHER  "MusicForge contributors"
!define EXE_GUI    "musicforge-gui.exe"
!define EXE_CLI    "musicforge.exe"
!define UNINSTKEY  "Software\Microsoft\Windows\CurrentVersion\Uninstall\MusicForge"
!define REGKEY     "Software\MusicForge"
!define DISTDIR    "..\dist\musicforge-v${VERSION}-windows-x64"

Name                "${APPNAME} ${VERSION}"
OutFile             "MusicForge-${VERSION}-setup.exe"
BrandingText        "${APPDESC}"
InstallDir          "$LOCALAPPDATA\Programs\MusicForge"
InstallDirRegKey    HKCU "${REGKEY}" "InstallDir"

RequestExecutionLevel user
SetCompressor       /SOLID lzma
SetCompressorDictSize 64
ShowInstDetails     show
ShowUnInstDetails   show

VIProductVersion              "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APPDESC}"
VIAddVersionKey "CompanyName"    "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "MIT License. Copyright (c) 2026 ${PUBLISHER}"

; ------------------------------------------------------------------- 页面 --
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "欢迎安装 MusicForge ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT  "MusicForge 是一个完全离线的本地音频格式转换工具。$\r$\n$\r$\n它不会连接互联网、不会上传任何文件、也不会收集任何数据。$\r$\n$\r$\n请仅将其用于处理你已合法获得的文件的个人本地转换。$\r$\n$\r$\n本安装程序无需管理员权限，只安装给当前用户。$\r$\n$\r$\n建议在安装前关闭正在运行的 MusicForge。"

!define MUI_FINISHPAGE_RUN              "$INSTDIR\${EXE_GUI}"
!define MUI_FINISHPAGE_RUN_TEXT         "立即启动 MusicForge 图形界面"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!define MUI_FINISHPAGE_SHOWREADME              "$INSTDIR\使用说明.txt"
!define MUI_FINISHPAGE_SHOWREADME_TEXT         "查看使用说明"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "license-setup.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!define MUI_UNCONFIRMPAGE_TEXT_TOP \
  "将从下列位置卸载 MusicForge：$\r$\n$INSTDIR$\r$\n$\r$\n卸载只会移除程序本体与快捷方式，不会删除你已转换的音频文件。"

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"

; --------------------------------------------------------------- 安装段 --
Section "MusicForge 主程序（必需）" SEC_MAIN
  SectionIn RO

  SetOutPath "$INSTDIR"

  File /oname=${EXE_GUI}      "${DISTDIR}\${EXE_GUI}"
  File /oname=${EXE_CLI}      "${DISTDIR}\${EXE_CLI}"
  File /oname=使用说明.txt     "${DISTDIR}\使用说明.txt"
  File /oname=LICENSE.txt     "${DISTDIR}\LICENSE"
  File /oname=README.md       "${DISTDIR}\README.md"

  DetailPrint "写入卸载信息……"
  WriteRegStr   HKCU "${UNINSTKEY}" "DisplayName"     "${APPNAME} ${VERSION}"
  WriteRegStr   HKCU "${UNINSTKEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${UNINSTKEY}" "Publisher"       "${PUBLISHER}"
  WriteRegStr   HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr   HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr   HKCU "${UNINSTKEY}" "DisplayIcon"     "$INSTDIR\${EXE_GUI},0"
  WriteRegStr   HKCU "${UNINSTKEY}" "HelpLink"        "https://github.com/simenty/MusicForge/issues"
  WriteRegStr   HKCU "${UNINSTKEY}" "URLInfoAbout"    "https://github.com/simenty/MusicForge"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify"        1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair"        1
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize"   4300
  WriteRegStr   HKCU "${REGKEY}"    "InstallDir"      "$INSTDIR"
  WriteRegStr   HKCU "${REGKEY}"    "Version"         "${VERSION}"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd

Section "开始菜单快捷方式" SEC_STARTMENU
  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\MusicForge"
  CreateShortCut "$SMPROGRAMS\MusicForge\MusicForge.lnk"        "$INSTDIR\${EXE_GUI}" "" "$INSTDIR\${EXE_GUI}" 0
  CreateShortCut "$SMPROGRAMS\MusicForge\使用说明.lnk"      "$INSTDIR\使用说明.txt"
  CreateShortCut "$SMPROGRAMS\MusicForge\卸载 MusicForge.lnk"    "$INSTDIR\Uninstall.exe"
SectionEnd

Section /o "桌面快捷方式" SEC_DESKTOP
  SetShellVarContext current
  CreateShortCut "$DESKTOP\MusicForge.lnk" "$INSTDIR\${EXE_GUI}" "" "$INSTDIR\${EXE_GUI}" 0
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_MAIN}      "MusicForge 主程序：图形界面 musicforge-gui.exe + 命令行 musicforge.exe。两者都完全离线运行。"
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_STARTMENU} "在「开始」菜单中创建 MusicForge 文件夹与快捷方式。"
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_DESKTOP}   "在桌面创建快捷方式（默认不创建）。"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; --------------------------------------------------------------- 卸载段 --
Section "Uninstall"
  SetShellVarContext current

  Delete "$INSTDIR\${EXE_GUI}"
  Delete "$INSTDIR\${EXE_CLI}"
  Delete "$INSTDIR\使用说明.txt"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\Uninstall.exe"

  Delete "$DESKTOP\MusicForge.lnk"
  RMDir /r "$SMPROGRAMS\MusicForge"

  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${REGKEY}"
SectionEnd

; ------------------------------------------------------------ 回调函数 --
; 注意：NSIS 在静默模式（/S）下不会显示 MessageBox，且对含「取消」按钮的
; 消息框会直接返回 Cancel。因此所有确认框必须用 IfSilent 跳过，
; 否则 /S 时会被 Abort 吞掉，表现为「exit 0 但什么都没做」。
Function .onInit
  SetShellVarContext current

  ; 若已安装旧版本，先卸载，避免文件残留或版本混杂
  ReadRegStr $0 HKCU "${UNINSTKEY}" "UninstallString"
  StrCmp $0 "" no_previous

  IfSilent do_uninstall
  MessageBox MB_OKCANCEL|MB_ICONQUESTION \
    "检测到本机已安装 MusicForge，需要先卸载旧版本才能继续。$\r$\n$\r$\n是否现在卸载旧版本并继续安装？" \
    IDOK do_uninstall
  Abort

  do_uninstall:
    ReadRegStr $1 HKCU "${UNINSTKEY}" "InstallLocation"
    ${If} $1 != ""
      ExecWait '$0 /S _?=$1'
    ${Else}
      ExecWait '$0 /S'
    ${EndIf}

  no_previous:
FunctionEnd

; 卸载确认由 MUI_UNPAGE_CONFIRM 页面负责，此处不再二次弹窗，
; 以便静默卸载（/S）能够真正执行。
Function un.onInit
  SetShellVarContext current
FunctionEnd
