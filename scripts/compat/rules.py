"""VS Code 命令 / 设置 → Rove 判定规则（依据 docs/FEATURES.md 与 docs/UX_SPEC.md）。

每条规则：(正则, 判定, 优先级, 功能区, 说明)。按顺序匹配，第一条命中的规则生效。
判定：impl（实现）、no（不做）、ext（来自用户安装的扩展，不属于 VS Code 默认，排除）。
"""

# 用户安装的扩展（不是 VS Code 自带），在清单中整体排除
EXT_NAMESPACES = (
    "gitlens", "csdevkit", "dotnet", "csharp", "omnisharp", "razor", "dotnetAcquisitionExtension",
    "nuget", "markdown-mermaid", "remote-wsl", "microsoft-sovereign-cloud",
)

NO = "no"
IMPL = "impl"

# ---------------------------------------------------------------- 键位命令规则
COMMAND_RULES = [
    # —— 不做：整个功能域 ——
    (r"^(notebook|interactive|repl|testing|debug|breakpointWidget|closeBreakpointWidget|scm|remote|mergeEditor|multiDiffEditor|"
     r"chat|chatEditing|chatEditor|agentSession|agentsVoice|inlineChat2?|refactorPreview|extension|o)\.", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    (r"^workbench\.action\.(terminal|debug|tasks|chat|remote|extensions|notebook|testing|interactive|tunnels|speech|voice|"
     r"inlineChat|agent|output|toggleDevTools|reportIssue|openIssueReporter|showRuntimeExtensions|showEmmetCommands)", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    (r"^workbench\.(debug|chat|banner)\.", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    (r"^workbench\.view\.(scm|debug|extensions|testing|remote)", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    (r"^editor\.(debug|action\.(dirtydiff|accessibleDiffViewer|showDebugHover|toggleBreakpoint))|^editor\.(toggleBreakpoint|toggleInlineBreakpoint|closeExceptionWidget|showDebugHover|closeTestPeek)", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    (r"^(closeQuickDiff|editor\.action\.inlineSuggest|editor\.action\.accessibleViewAcceptInlineCompletion)", NO, "", "调试/终端/SCM/AI/扩展", "FEATURES §10"),
    # —— 不做：实时语言服务 ——
    (r"(Suggest|Suggestion|suggest)", NO, "", "实时语言服务", "FEATURES §4：不做补全"),
    (r"(ParameterHint|parameterHints)", NO, "", "实时语言服务", "FEATURES §4：不做参数提示"),
    (r"(RenameInput|^editor\.action\.rename$|^markdown\.editor\.rename)", NO, "", "实时语言服务", "FEATURES §4：不做重命名重构"),
    (r"(CodeAction|codeAction|quickFix|refactor|sourceAction|autoFix|organizeImports|fixAll)", NO, "", "实时语言服务", "FEATURES §4：不做代码动作"),
    (r"(Snippet|snippet|emmet|expandAbbreviation)", NO, "", "实时语言服务", "FEATURES §4：不做代码片段"),
    (r"(ColorPicker|colorPicker)", NO, "", "实时语言服务", "FEATURES §4：不做取色器"),
    (r"^editor\.action\.inPlaceReplace", NO, "", "实时语言服务", "FEATURES §4"),
    (r"(inlayHints|InlayHint)", NO, "", "实时语言服务", "FEATURES §5：内嵌提示待议"),
    (r"(comments|Comment(Thread|ing|edRange))", NO, "", "调试/终端/SCM/AI/扩展", "VS Code 的评论功能（不是注释切换）"),
    # —— Markdown ——
    (r"^markdown\.editor\.", NO, "", "Markdown", "VS Code 1.145 新增的所见即所得 Markdown 编辑器；Rove 不做，保持“预览 + 源码”（D7）"),
    (r"^markdown\.(togglePreview|showPreviewToSide|showPreview|showSource|showLockedPreviewToSide|toggleLock|preview)", IMPL, "P0", "Markdown", "UX_SPEC §4.3；D6 的切换直接使用 `markdown.togglePreview`"),
    # —— 无障碍 ——
    (r"(accessib|Accessib|announce|accessibleView|widgetNavigation|toggleTabFocusMode|noteMultiCursor|stopReadingLine)", IMPL, "P2", "无障碍", "无障碍功能，P2"),
    # —— 光标与基础编辑（M3） ——
    (r"^(cursor[A-Z]|cursorUndo|cursorRedo|deleteLeft|deleteRight|deleteWord|deleteAllLeft|deleteAllRight|cancelSelection|removeSecondaryCursors|tab$|outdent$|lineBreakInsert|undo$|redo$|default:type|type$|scroll(Line|Page)(Up|Down)|expandLineSelection)", IMPL, "P0", "编辑基础", "FEATURES §4"),
    (r"^editor\.action\.(clipboard|selectAll|insertLineAfter|insertLineBefore|indentLines|outdentLines|setSelectionAnchor|selectFromAnchorToCursor|cancelSelectionAnchor|goToSelectionAnchor|pasteAsText)", IMPL, "P0", "编辑基础", "FEATURES §4"),
    (r"^(editor\.action\.(changePasteType|changeDropType|dropIntoEditor))|^editor\.(changePasteType|changeDropType|cancelOperation)", IMPL, "P2", "编辑基础", "拖放与粘贴类型选择，P2"),
    # —— 多光标、行操作、注释、括号、扩选 ——
    (r"^editor\.action\.(insertCursor|addSelectionTo|moveSelectionTo|selectHighlights|changeAll|addCursorsAt|selectAllMatches)|^addCursorsAtSearchResults|^cursorColumnSelect", IMPL, "P1", "多光标", "FEATURES §4"),
    (r"^editor\.action\.(copyLines|moveLines|deleteLines|joinLines|sortLines|transpose|transformTo|trimTrailingWhitespace|duplicateSelection)", IMPL, "P1", "行操作", "FEATURES §4"),
    (r"^editor\.action\.(commentLine|addCommentLine|removeCommentLine|blockComment)", IMPL, "P1", "注释", "FEATURES §4"),
    (r"^editor\.action\.(jumpToBracket|selectToBracket|removeBrackets)", IMPL, "P1", "括号", "FEATURES §3"),
    (r"^editor\.action\.smartSelect", IMPL, "P1", "智能扩选", "FEATURES §4"),
    (r"^editor\.action\.(formatDocument|formatSelection)", IMPL, "P1", "格式化", "仅 JSON/XML（FEATURES §4）"),
    (r"^editor\.action\.linkedEditing|^cancelLinkedEditingInput", IMPL, "P2", "编辑基础", "XML 标签联动编辑，P2"),
    # —— 查找替换 ——
    (r"^(actions\.find|closeFindWidget|toggleFind|toggleCaseSensitive|toggleWholeWord|toggleRegex|togglePreserveCase|toggleSearch)|^editor\.action\.(nextMatchFindAction|previousMatchFindAction|nextSelectionMatchFindAction|previousSelectionMatchFindAction|startFindReplaceAction|replaceOne|replaceAll|findWithSelection|toggleFindInSelection|findReferencesInFiles)", IMPL, "P0", "查找替换", "FEATURES §4"),
    # —— 导航与语言智能 ——
    (r"^editor\.action\.(revealDefinition|goToDeclaration|goToTypeDefinition|peekDefinition|goToReferences|referenceSearch|showHover|goToTopHover|goToBottomHover|pageUpHover|pageDownHover|scroll(Up|Down|Left|Right)Hover|increaseHoverVerbosity|decreaseHoverVerbosity)|^closeReferenceSearch|^editor\.(gotoNextSymbolFromResult|gotoPreviousSymbolFromResult)|^references-view\.(?!showCallHierarchy|showTypeHierarchy)", IMPL, "P0", "导航", "FEATURES §5；查找引用默认用速览（D3）"),
    (r"^editor\.action\.(revealDefinitionAside|goToImplementation|peekImplementation|wordHighlight)", IMPL, "P1", "导航", "FEATURES §5"),
    (r"(CallHierarchy|callHierarchy|TypeHierarchy|typeHierarchy)", IMPL, "P2", "导航", "调用层级 / 类型层级，P2"),
    (r"^editor\.action\.marker\.|^closeMarkersNavigation", IMPL, "P1", "问题", "只用于 JSON/YAML/XML 的语法错误（FEATURES §5）"),
    (r"^editor\.action\.(goToFocusedStickyScrollLine|selectPreviousStickyScrollLine|selectNextStickyScrollLine|focusStickyScroll|toggleStickyScroll)", IMPL, "P1", "阅读", "粘性滚动（FEATURES §3）"),
    (r"^editor\.(fold|unfold|toggleFold|foldAll|unfoldAll|foldLevel|foldRecursively|unfoldRecursively|foldAllBlockComments|foldAllMarkerRegions|unfoldAllMarkerRegions|foldAllExcept|unfoldAllExcept|gotoParentFold|gotoNextFold|gotoPreviousFold|createFoldingRangeFromSelection|removeManualFoldingRanges|toggleFoldRecursively)", IMPL, "P0", "阅读", "折叠（FEATURES §3）"),
    (r"^editor\.action\.(fontZoom|toggleWordWrap|toggleRenderWhitespace|toggleMinimap|toggleRenderControlCharacter)", IMPL, "P0", "阅读", "FEATURES §3"),
    (r"^editor\.(detectLanguage)", IMPL, "P1", "阅读", "语言检测（FEATURES §5）"),
    (r"^editor\.action\.(selectEditor|defineKeybinding)", IMPL, "P2", "设置", "键位编辑器相关，P2"),
    (r"^breadcrumbs\.", IMPL, "P0", "阅读", "面包屑"),
    # —— 快速访问 ——
    (r"^workbench\.action\.(quickOpen|showCommands|gotoLine|gotoSymbol|quickOpenNavigate|quickOpenSelect|closeQuickOpen|acceptSelectedQuickOpenItem|alternativeAcceptSelectedQuickOpenItem|focusQuickOpen|quickInputBack|showAllEditors|quickOpenView|openRecent|openPreviousRecentlyUsedEditor|openNextRecentlyUsedEditor|quickOpenPreviousRecentlyUsedEditor|quickOpenLeastRecentlyUsedEditor|quickOpenPreviousEditorFromHistory|quickOpenRecent|quickTextSearch)|^quickInput\.", IMPL, "P0", "快速访问", "FEATURES §6"),
    (r"^workbench\.action\.showAllSymbols", IMPL, "P1", "快速访问", "工作区符号（FEATURES §5）"),
    # —— 搜索 ——
    (r"^workbench\.action\.(findInFiles|replaceInFiles|clearSearchResults|focusNextSearchResult|focusPreviousSearchResult|toggleSearchEditor|search)|^search\.action\.(?!replace)|^search\.(focus|toggle|searchEditor)|^closeReplaceInFilesWidget", IMPL, "P0", "搜索", "FEATURES §6"),
    (r"^search\.action\.replace|^workbench\.action\.replaceInFiles", IMPL, "P1", "搜索", "全局替换（FEATURES §6）"),
    (r"(SearchEditor|searchEditor)", IMPL, "P2", "搜索", "搜索编辑器（FEATURES §6）"),
    # —— 文件与资源管理器 ——
    (r"^workbench\.action\.files\.(save|saveAs|saveAll|saveWithoutFormatting|revert|newUntitledFile|openFile|openFolder|openFileFolder|copyPathOfActiveFile|showActiveFileInExplorer|revealActiveFileInWindows|compareWithSaved|compareFileWith)", IMPL, "P0", "文件", "FEATURES §6"),
    (r"^workbench\.files\.action\.(compareWithSaved|compareFileWith)", IMPL, "P2", "文件", "对比视图（FEATURES §6）"),
    (r"^workbench\.files\.action\.|^explorer\.|^workbench\.view\.explorer|^workbench\.explorer", IMPL, "P0", "文件", "资源管理器（FEATURES §6）"),
    (r"^(filesExplorer\.|renameFile|deleteFile|moveFileToTrash|copyFilePath|copyRelativeFilePath|revealFileInOS|explorer\.newFile|explorer\.newFolder|workbench\.action\.files\.(newFile|newFolder))", IMPL, "P1", "文件", "资源管理器中的文件操作（FEATURES §6）"),
    # —— 编辑器组、布局、窗口 ——
    (r"^workbench\.action\.(closeActiveEditor|closeAllEditors|closeOtherEditors|closeEditorsInGroup|closeEditorsToTheRight|closeEditorsToTheLeft|closeUnmodifiedEditors|closeGroup|closeAllGroups|closeEditorInAllGroups|nextEditor|previousEditor|nextEditorInGroup|previousEditorInGroup|openEditorAtIndex|lastEditorInGroup|firstEditorInGroup|splitEditor|focus.*EditorGroup|focus(First|Second|Third|Fourth|Fifth|Sixth|Seventh|Eighth|Next|Previous|Left|Right|Above|Below)Group|focusActiveEditorGroup|moveEditor|moveActiveEditor|pinEditor|unpinEditor|keepEditor|reopenClosedEditor|navigateEditorGroups|toggleEditorGroupLayout|editorLayout|evenEditorWidths|toggleMaximizeEditorGroup|minimizeOtherEditors|joinTwoGroups|newGroup|moveEditorToNewWindow|lockEditorGroup|unlockEditorGroup|toggleEditorGroupLock|showEditorsInActiveGroup|duplicateActiveEditor|copyEditorToNewWindow|increaseViewSize|decreaseViewSize|focusActiveEditor|openEditorAtIndex.*)", IMPL, "P0", "编辑器组", "FEATURES §2"),
    (r"^workbench\.action\.(navigateBack|navigateForward|navigateToLastEditLocation|navigateLast|navigateBackInEditLocations|navigateForwardInEditLocations|navigatePreviousInEditLocations|navigateBackInNavigationLocations|navigateForwardInNavigationLocations|navigateToLastNavigationLocation|navigateLeft|navigateRight|navigateUp|navigateDown)", IMPL, "P0", "导航", "导航历史（FEATURES §5）"),
    (r"^workbench\.action\.(toggleSidebarVisibility|togglePanel|focusSideBar|focusPanel|closePanel|toggleMaximizedPanel|toggleStatusbarVisibility|toggleActivityBarVisibility|focusActivityBar|focusStatusBar|toggleSidebarPosition|focusTitleBar|toggleCenteredLayout|toggleZenMode|exitZenMode|toggleFullScreen|nextSideBarView|previousSideBarView|nextPanelView|previousPanelView|focusNextPart|focusPreviousPart|toggleMenuBar|showInterfaceOverview|toggleAuxiliaryBar|focusAuxiliaryBar|closeAuxiliaryBar)", IMPL, "P0", "布局", "FEATURES §2"),
    (r"^workbench\.action\.(newWindow|closeWindow|closeFolder|reloadWindow|quit|zoomIn|zoomOut|zoomReset|openSettings|openGlobalSettings|openSettingsJson|openGlobalKeybindings|openGlobalKeybindingsFile|openDefaultKeybindingsFile|selectTheme|selectIconTheme|configureLocale|toggleLightDarkThemes|showAboutDialog|openWorkspaceSettings|keybindingsReference|openDocumentationUrl|showCommands|focusFilesExplorer)", IMPL, "P0", "窗口与设置", "FEATURES §7"),
    (r"^workbench\.action\.(showErrorsWarnings|problems)|^problems\.|^workbench\.panel\.markers", IMPL, "P1", "问题", "FEATURES §5"),
    (r"^workbench\.view\.(search|explorer)", IMPL, "P0", "布局", "FEATURES §2"),
    (r"^workbench\.(statusBar|actions\.view|view\.|panel\.)", IMPL, "P0", "布局", "FEATURES §2"),
    (r"^views\.moveView", IMPL, "P2", "布局", "视图拖动重排（FEATURES §2）"),
    (r"^(notifications|notification)\.", IMPL, "P0", "通知", "UX_SPEC §6.1"),
    (r"^(list|history|widgetNavigation)\.", IMPL, "P0", "通用列表", "列表、树、输入历史的通用键盘导航，资源管理器、搜索、大纲都依赖它"),
    (r"^settings\.", IMPL, "P1", "设置", "图形化设置界面（FEATURES §7）"),
    (r"^keybindings\.", IMPL, "P2", "设置", "图形化键位编辑器（FEATURES §7）"),
    (r"^welcome\.", IMPL, "P0", "欢迎页", "UX_SPEC §1.6（只保留开始与最近）"),
    (r"^diffEditor\.|^workbench\.action\.compareEditor|^editor\.action\.diffReview", IMPL, "P2", "对比", "对比视图（FEATURES §6）"),
    (r"^workbench\.edit\.", IMPL, "P1", "编辑基础", ""),
    (r"^(closeBreakpointWidget|editor\.action\.(extensioneditor|webvieweditor|submitComment)|workbench\.action\.(browser|addComment|hideComment|editorDictation|issueReporter|openAgentsWindow|openChat|quickchat|files\.openLocalFile|files\.openLocalFolder|closeModalEditor|navigateModalEditor)|insertBestCompletion|toggleExplainMode|toggleQuickDiffWidgetFocus)", NO, "", "调试/终端/SCM/AI/扩展",
     "扩展页、WebView、内置浏览器、评论、听写、问题上报、AI、远程文件对话框、模态编辑器等，均不做"),
    (r"^(goToNextReference|goToPreviousReference|openReferenceToSide|revealReference|togglePeekWidgetFocus)", IMPL, "P0", "导航", "速览窗口内的操作（D3）"),
    (r"^(editor\.action\.showContextMenu|leaveEditorMessage|saveAll|workbench\.action\.(showHover|editor\.changeLanguageMode|joinEditorInGroup))", IMPL, "P0", "编辑器组", ""),
    (r"^(firstCompressedFolder|lastCompressedFolder|nextCompressedFolder|previousCompressedFolder)", IMPL, "P0", "文件", "资源管理器的紧凑文件夹导航"),
    (r"^editor\.action\.toggleOvertypeInsertMode", IMPL, "P1", "编辑基础", "Insert 键切换改写模式"),
    (r"^(editor\.(hideDropWidget|hidePasteWidget|showIncomingCalls|showOutgoingCalls|showSubtypes|showSupertypes)|workbench\.action\.(editor\.nextChange|editor\.previousChange|restoreAuxiliaryBar))", IMPL, "P2", "导航", "调用/类型层级、对比视图中的变更导航、辅助侧边栏，P2"),
    (r"^workbench\.action\.(togglePreserveCase|toggleScreencastMode|logStorage|inspectContextKeys|inspectKeyMappings|showLogs|openLogFile|openProcessExplorer|toggleKeybindingsLog)", IMPL, "P2", "开发者", "开发者工具类命令"),
]

# ---------------------------------------------------------------- 设置规则
SETTING_RULES = [
    (r"^\[", IMPL, "P0", "语言级覆盖", "支持语言级覆盖语法（D4 第 13 节规则 2）；只对 Rove 支持的设置项生效"),
    # 不做的命名空间
    (r"^launch$|^(chat|agents|inlineChat|dictation|terminal|debug|launch|task|testing|notebook|interactiveWindow|ipynb|scm|git|github|github-authentication|github-enterprise|microsoft-authentication|remote|extensions|telemetry|update|settingsSync|sessions|comments|timeline|merge-conflict|mergeEditor|issueReporter|onboarding|imageCarousel|emmet|css|less|scss|html|php|npm|grunt|gulp|jake|simpleBrowser|output|security|http|network|application)\.", NO, "", "不做的功能域", "FEATURES §10"),
    # 逐项排除：AI、模态编辑器、新视觉特效、依赖语言服务或扩展的零散设置项
    (r"^(editor\.(snippets|rename|defaultFoldingRangeProvider)|workbench\.(commandPalette\.showAskInChat|editor\.markdownDefaultEditorInAgentsWindow|editor\.useModal|modernUIFrostedGlass|modernUIFrostedGlassOpacity|shadows|reduceTransparency|settings\.(showAISearchToggle|enableNaturalLanguageSearch|applyToAllProfiles)|welcomePage\.experimentalOnboarding|editorLargeFileConfirmation))",
     NO, "", "零散排除", "AI、模态编辑器、视觉特效、依赖语言服务或扩展的设置；`workbench.editorLargeFileConfirmation` 由 Rove 自己的大文件分级取代（UX_SPEC §3.1）"),
    (r"^editor\.renderRichScreenReaderContent|^workbench\.editor\.dragToOpenWindow", IMPL, "P2", "编辑器", "P2"),
    # 编辑器：不做
    (r"^editor\.(suggest|quickSuggestions|acceptSuggestion|suggestOnTrigger|suggestSelection|suggestFontSize|suggestLineHeight|tabCompletion|snippetSuggestions|wordBasedSuggestions|inlineSuggest|inlineCompletions|parameterHints|codeLens|codeActions|lightbulb|inlayHints|colorDecorators|defaultColorDecorators|formatOnType|renameOnType|pasteAs|dropIntoEditor|experimental|gotoLocation\.alternative|codeActionWidget|occurrencesHighlightDelay|editContext|aiStats|inlineCompletionsAccessibilityVerbose|screenReaderAnnounceInlineSuggestion|showDeprecated|showUnused|foldingImportsByDefault|linkedEditing)", NO, "", "实时语言服务", "FEATURES §4/§5"),
    (r"^editor\.(minimap|stickyScroll|bracketPairColorization|guides\.bracketPairs|multiCursor|columnSelection|wordWrap|wrappingIndent|wrappingStrategy|rulers|autoClosing|autoIndent|autoSurround|trimAutoWhitespace|formatOnSave|formatOnPaste|defaultFormatter|semanticHighlighting|semanticTokenColorCustomizations|links|overviewRuler|hideCursorInOverviewRuler|smartSelect|comments|insertSpaces|tabSize|detectIndentation|indentSize|useTabStops|cursorSmoothCaretAnimation|smoothScrolling|definitionLinkOpensInPeek)", IMPL, "P1", "编辑器", "FEATURES §3/§4"),
    (r"^editor\.(unicodeHighlight|renderControlCharacters|unusualLineTerminators|dragAndDrop|copyWithSyntaxHighlighting|accessibility|ariaLabel|ariaRequired|screenReader|accessibilitySupport|accessibilityPageSize|tabFocusMode|middleClick|selectionClipboard|mouseMiddleClickAction)", IMPL, "P2", "编辑器", "P2"),
    (r"^editor\.", IMPL, "P0", "编辑器", "外观、光标、查找、悬停、折叠、滚动等基础行为（FEATURES §3）"),
    (r"^diffEditor\.|^multiDiffEditor\.", IMPL, "P2", "对比", "FEATURES §6"),
    # 工作台
    (r"^workbench\.(localHistory)", IMPL, "P2", "工作台", "FEATURES §6"),
    (r"^workbench\.(settings)", IMPL, "P1", "设置", "图形化设置界面"),
    (r"^workbench\.(productIconTheme|cloudChanges|experimental|enableExperiments|tips|commandPalette\.experimental|welcomePage\.walkthroughs|remoteIndicator|externalBrowser|externalUriOpeners|trustedDomains|browser|secondarySideBar|auxiliaryBar|panel\.opensMaximized|layoutControl\.type|navigationControl|view\.showQuietly|startupEditor\.walkthrough|editor\.empty|editor\.historyBasedLanguageDetection|editor\.languageDetection|editor\.preferHistoryBasedLanguageDetection|editor\.autoLockGroups|accounts|reduceMotion|hover\.reduceMotion|activityBar\.iconClickBehavior|chat|agent|sash\.hoverDelay)", NO, "", "工作台", "不做或无对应功能"),
    (r"^workbench\.", IMPL, "P0", "工作台", "主题、标签、布局、列表、命令面板等（FEATURES §2/§7）"),
    (r"^window\.(zoomLevel|zoomPerWindow|restoreWindows|openFoldersInNewWindow|openFilesInNewWindow|confirmBeforeClose|newWindowDimensions|menuBarVisibility|autoDetectColorScheme|autoDetectHighContrast|systemColorTheme|confirmSaveUntitledWorkspace|closeWhenEmpty|enableMenuBarMnemonics|customMenuBarAltFocus|doubleClickIconToClose)", IMPL, "P1", "窗口", "FEATURES §2"),
    (r"^window\.", IMPL, "P0", "窗口", "标题、标题栏样式、命令中心等（只支持自定义标题栏）"),
    (r"^files\.(hotExit|insertFinalNewline|trimTrailingWhitespace|trimFinalNewlines|autoSave|enableTrash|restoreUndoStack|saveConflictResolution|defaultLanguage|dialog)", IMPL, "P1", "文件", "FEATURES §4/§6"),
    (r"^files\.(simpleDialog|participants|refactoring|maxMemoryForLargeFilesMB|experimental)", NO, "", "文件", "Rove 不适用（超大文件由 Rove 自己的分级处理，UX_SPEC §3.1）"),
    (r"^files\.", IMPL, "P0", "文件", "编码、换行符、排除、关联、只读、监听（UX_SPEC §3）"),
    (r"^search\.(useIgnoreFiles|useGlobalIgnoreFiles|useParentIgnoreFiles)", NO, "", "搜索", "Rove 不读取 .gitignore（FEATURES §6）；这些设置被忽略"),
    (r"^search\.(searchEditor|experimental|searchView|defaultViewMode)", IMPL, "P2", "搜索", "搜索编辑器等，P2"),
    (r"^search\.", IMPL, "P0", "搜索", "FEATURES §6"),
    (r"^explorer\.(excludeGitIgnore|decorations\.badges|decorations\.colors)", NO, "", "资源管理器", "依赖 Git 或扩展装饰"),
    (r"^explorer\.(openEditors|fileNesting)", IMPL, "P2", "资源管理器", "P2"),
    (r"^explorer\.", IMPL, "P0", "资源管理器", "FEATURES §6"),
    (r"^(outline|breadcrumbs)\.", IMPL, "P0", "大纲与面包屑", "FEATURES §3/§5"),
    (r"^problems\.", IMPL, "P1", "问题", "FEATURES §5"),
    (r"^zenMode\.", IMPL, "P1", "布局", "FEATURES §2"),
    (r"^mediaPreview\.", IMPL, "P1", "图片预览", "FEATURES §6"),
    (r"^references\.", IMPL, "P1", "导航", "引用视图（备选）"),
    (r"^json\.(schemas|schemaDownload|colorDecorators|validate\.enable)", NO, "", "JSON", "不做 JSON Schema（FEATURES §9）"),
    (r"^json\.", IMPL, "P1", "JSON", "格式化、大纲（FEATURES §9）"),
    (r"^markdown\.(validate|updateLinksOnFileMove|suggest|editor\.(drop|paste|filePaste|updateLinks)|server|experimental|occurrencesHighlight|copyFiles)", NO, "", "Markdown", "依赖 Markdown 语言服务的链接校验、补全、文件拖放等，不做"),
    (r"^markdown\.", IMPL, "P0", "Markdown", "预览与样式（UX_SPEC §4.3、D5）"),
    (r"^(accessibility|screencastMode)\.", IMPL, "P2", "无障碍", "P2"),
    (r"^keyboard\.", IMPL, "P0", "键位", "键位分发方式（D4）"),
]
