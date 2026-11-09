# 键位兼容清单（阶段 0.3a）

> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code 1.145.2（Windows，中文界面导出）的默认键位 · 生成脚本：`scripts/compat/gen_compat_docs.py` · 判定规则：`scripts/compat/rules.py`

## 1. 说明

- Rove 与 VS Code 共用 `keybindings.json`（D4），命令 ID 一律沿用 VS Code。下表中“实现”的条目，Rove 必须以**相同的命令 ID、相同的默认键位、相同的 `when` 条件**提供。
- 判定依据是 `FEATURES.md` 与 `UX_SPEC.md`。对判定有异议时，修改 `rules.py` 后重新生成，不要直接改本文件。
- 导出文件中出现的 `oem_2`、`oem_5` 等是 Windows 虚拟键码名（与键盘布局有关）。Rove 按 VS Code 的 `keyboard.dispatch` 规则解析，保持一致。
- 用户安装的扩展（GitLens、C# Dev Kit 等）贡献的键位不属于 VS Code 默认，已排除。

## 2. 统计

| 判定 | 条数 |
|---|---|
| 实现 P0 | 435 |
| 实现 P1 | 82 |
| 实现 P2 | 74 |
| 不做 | 618 |
| 扩展贡献（排除） | 60 |
| **合计** | 1269 |

另外，导出文件末尾还列出了 3450 个没有默认键位的命令。它们不影响键位兼容；实现某个功能时，相应命令的 ID 同样沿用 VS Code。

## 3. 需要实现的键位（按功能区）

### Markdown（2）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `ctrl+k v` | `markdown.showPreviewToSide` | `editorFocus && !notebookEditorFocused && editorLangId =~ /^(markdown\|prompt\|instructions\|chatagent\|skill)$/` |
| P0 | `ctrl+shift+v` | `markdown.togglePreview` | `!terminalFocus && activeCustomEditorId == 'vscode.markdown.preview.editor' \|\| editorFocus && !notebookEditorFocused && !terminalFocus && resourceLangId =~ /^(markdown\|prompt\|instructions\|chatagent\|skill)$/` |

### 导航（60）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `escape` | `closeReferenceSearch` | `inReferenceSearchEditor && !config.editor.stablePeek` |
| P0 | `escape` | `closeReferenceSearch` | `editorTextFocus && referenceSearchVisible && !config.editor.stablePeek \|\| referenceSearchVisible && !config.editor.stablePeek && !inputFocus` |
| P0 | `shift+escape` | `closeReferenceSearch` | `inReferenceSearchEditor && !config.editor.stablePeek` |
| P0 | `shift+escape` | `closeReferenceSearch` | `editorTextFocus && referenceSearchVisible && !config.editor.stablePeek \|\| referenceSearchVisible && !config.editor.stablePeek && !inputFocus` |
| P0 | `ctrl+down` | `editor.action.goToBottomHover` | `editorHoverFocused` |
| P0 | `end` | `editor.action.goToBottomHover` | `editorHoverFocused` |
| P0 | `shift+f12` | `editor.action.goToReferences` | `editorHasReferenceProvider && editorTextFocus && !inReferenceSearchEditor && !isInEmbeddedEditor` |
| P0 | `ctrl+up` | `editor.action.goToTopHover` | `editorHoverFocused` |
| P0 | `home` | `editor.action.goToTopHover` | `editorHoverFocused` |
| P0 | `alt+down` | `editor.action.pageDownHover` | `editorHoverFocused` |
| P0 | `pagedown` | `editor.action.pageDownHover` | `editorHoverFocused` |
| P0 | `alt+up` | `editor.action.pageUpHover` | `editorHoverFocused` |
| P0 | `pageup` | `editor.action.pageUpHover` | `editorHoverFocused` |
| P0 | `alt+f12` | `editor.action.peekDefinition` | `editorHasDefinitionProvider && editorTextFocus && !inReferenceSearchEditor && !isInEmbeddedEditor` |
| P0 | `ctrl+f12` | `editor.action.revealDefinition` | `editorHasDefinitionProvider && editorTextFocus && isWeb` |
| P0 | `f12` | `editor.action.revealDefinition` | `editorHasDefinitionProvider && editorTextFocus` |
| P0 | `ctrl+k ctrl+f12` | `editor.action.revealDefinitionAside` | `editorHasDefinitionProvider && editorTextFocus && isWeb && !isInEmbeddedEditor` |
| P0 | `ctrl+k f12` | `editor.action.revealDefinitionAside` | `editorHasDefinitionProvider && editorTextFocus && !isInEmbeddedEditor` |
| P0 | `down` | `editor.action.scrollDownHover` | `editorHoverFocused` |
| P0 | `left` | `editor.action.scrollLeftHover` | `editorHoverFocused` |
| P0 | `right` | `editor.action.scrollRightHover` | `editorHoverFocused` |
| P0 | `up` | `editor.action.scrollUpHover` | `editorHoverFocused` |
| P0 | `ctrl+k ctrl+i` | `editor.action.showHover` | `editorTextFocus` |
| P0 | `f12` | `editor.gotoNextSymbolFromResult` | `hasSymbols` |
| P0 | `escape` | `editor.gotoNextSymbolFromResult.cancel` | `hasSymbols` |
| P0 | `f12` | `goToNextReference` | `inReferenceSearchEditor \|\| referenceSearchVisible` |
| P0 | `f4` | `goToNextReference` | `inReferenceSearchEditor \|\| referenceSearchVisible` |
| P0 | `shift+f12` | `goToPreviousReference` | `inReferenceSearchEditor \|\| referenceSearchVisible` |
| P0 | `shift+f4` | `goToPreviousReference` | `inReferenceSearchEditor \|\| referenceSearchVisible` |
| P0 | `ctrl+enter` | `openReferenceToSide` | `listFocus && referenceSearchVisible && !inputFocus && !treeElementCanCollapse && !treeElementCanExpand && !treestickyScrollFocused` |
| P0 | `shift+alt+f12` | `references-view.findReferences` | `editorHasReferenceProvider` |
| P0 | `f4` | `references-view.next` | `reference-list.hasResult && references-view.canNavigate` |
| P0 | `shift+f4` | `references-view.prev` | `reference-list.hasResult && references-view.canNavigate` |
| P0 | `enter` | `revealReference` | `listFocus && referenceSearchVisible && !inputFocus && !treeElementCanCollapse && !treeElementCanExpand && !treestickyScrollFocused` |
| P0 | `ctrl+k f2` | `togglePeekWidgetFocus` | `inReferenceSearchEditor \|\| referenceSearchVisible` |
| P0 | `alt+left` | `workbench.action.navigateBack` | `canNavigateBack` |
| P0 | `browserback` | `workbench.action.navigateBack` | `canNavigateBack` |
| P0 | `alt+right` | `workbench.action.navigateForward` | `canNavigateForward` |
| P0 | `browserforward` | `workbench.action.navigateForward` | `canNavigateForward` |
| P0 | `ctrl+k ctrl+q` | `workbench.action.navigateToLastEditLocation` |  |
| P1 | `ctrl+f12` | `editor.action.goToImplementation` | `editorHasImplementationProvider && editorTextFocus` |
| P1 | `ctrl+shift+f12` | `editor.action.peekImplementation` | `editorHasImplementationProvider && editorTextFocus && !inReferenceSearchEditor && !isInEmbeddedEditor` |
| P1 | `f7` | `editor.action.wordHighlight.next` | `editorTextFocus && hasWordHighlights` |
| P1 | `shift+f7` | `editor.action.wordHighlight.prev` | `editorTextFocus && hasWordHighlights` |
| P2 | `escape` | `editor.closeCallHierarchy` | `callHierarchyVisible && !config.editor.stablePeek` |
| P2 | `escape` | `editor.closeTypeHierarchy` | `typeHierarchyVisible && !config.editor.stablePeek` |
| P2 | `escape` | `editor.hideDropWidget` | `dropWidgetVisible` |
| P2 | `escape` | `editor.hidePasteWidget` | `pasteWidgetVisible` |
| P2 | `shift+enter` | `editor.refocusCallHierarchy` | `callHierarchyVisible` |
| P2 | `shift+enter` | `editor.refocusTypeHierarchy` | `typeHierarchyVisible` |
| P2 | `shift+alt+h` | `editor.showCallHierarchy` | `editorHasCallHierarchyProvider && editorTextFocus && !inReferenceSearchEditor` |
| P2 | `shift+alt+h` | `editor.showIncomingCalls` | `callHierarchyVisible && callHierarchyDirection == 'outgoingCalls'` |
| P2 | `shift+alt+h` | `editor.showOutgoingCalls` | `callHierarchyVisible && callHierarchyDirection == 'incomingCalls'` |
| P2 | `shift+alt+h` | `editor.showSubtypes` | `typeHierarchyVisible && typeHierarchyDirection == 'supertypes'` |
| P2 | `shift+alt+h` | `editor.showSupertypes` | `typeHierarchyVisible && typeHierarchyDirection == 'subtypes'` |
| P2 | `shift+alt+h` | `references-view.showCallHierarchy` | `editorHasCallHierarchyProvider` |
| P2 | `alt+f5` | `workbench.action.editor.nextChange` | `editorTextFocus && !textCompareEditorActive && quickDiffDecorationCount != '0'` |
| P2 | `shift+alt+f5` | `workbench.action.editor.previousChange` | `editorTextFocus && !textCompareEditorActive && quickDiffDecorationCount != '0'` |
| P2 | `ctrl+f4` | `workbench.action.restoreAuxiliaryBar` | `auxiliaryBarMaximized` |
| P2 | `ctrl+w` | `workbench.action.restoreAuxiliaryBar` | `auxiliaryBarMaximized` |

### 布局（23）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `escape escape` | `workbench.action.exitZenMode` | `inZenMode` |
| P0 | `f6` | `workbench.action.focusNextPart` |  |
| P0 | `shift+f6` | `workbench.action.focusPreviousPart` |  |
| P0 | `ctrl+0` | `workbench.action.focusSideBar` |  |
| P0 | `ctrl+alt+b` | `workbench.action.toggleAuxiliaryBar` |  |
| P0 | `f11` | `workbench.action.toggleFullScreen` | `!isIOS` |
| P0 | `ctrl+j` | `workbench.action.togglePanel` |  |
| P0 | `ctrl+b` | `workbench.action.toggleSidebarVisibility` |  |
| P0 | `ctrl+k z` | `workbench.action.toggleZenMode` | `!isAuxiliaryWindowFocusedContext` |
| P0 | `ctrl+shift+m` | `workbench.actions.view.problems` | `workbench.panel.markers.view.active` |
| P0 | `ctrl+alt+i` | `workbench.panel.chat` | `workbench.panel.chat.view.copilot.active` |
| P0 | `escape` | `workbench.statusBar.clearFocus` | `statusBarFocused` |
| P0 | `home` | `workbench.statusBar.focusFirst` | `statusBarFocused` |
| P0 | `end` | `workbench.statusBar.focusLast` | `statusBarFocused` |
| P0 | `down` | `workbench.statusBar.focusNext` | `statusBarFocused` |
| P0 | `right` | `workbench.statusBar.focusNext` | `statusBarFocused` |
| P0 | `left` | `workbench.statusBar.focusPrevious` | `statusBarFocused` |
| P0 | `up` | `workbench.statusBar.focusPrevious` | `statusBarFocused` |
| P0 | `ctrl+shift+f` | `workbench.view.search` | `workbench.view.search.active && neverMatch =~ /doesNotMatch/` |
| P2 | `ctrl+k down` | `views.moveViewDown` | `focusedView != ''` |
| P2 | `ctrl+k left` | `views.moveViewLeft` | `focusedView != ''` |
| P2 | `ctrl+k right` | `views.moveViewRight` | `focusedView != ''` |
| P2 | `ctrl+k up` | `views.moveViewUp` | `focusedView != ''` |

### 快速访问（63）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `alt+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `ctrl+alt+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `ctrl+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `ctrl+shift+alt+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `ctrl+shift+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `shift+alt+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `shift+enter` | `quickInput.accept` | `inQuickInput && quickInputType != 'quickWidget'` |
| P0 | `alt+right` | `quickInput.acceptInBackground` | `cursorAtEndOfQuickInputBox && inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && !inputFocus && quickInputType == 'quickPick'` |
| P0 | `ctrl+alt+right` | `quickInput.acceptInBackground` | `cursorAtEndOfQuickInputBox && inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && !inputFocus && quickInputType == 'quickPick'` |
| P0 | `ctrl+right` | `quickInput.acceptInBackground` | `cursorAtEndOfQuickInputBox && inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && !inputFocus && quickInputType == 'quickPick'` |
| P0 | `right` | `quickInput.acceptInBackground` | `cursorAtEndOfQuickInputBox && inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && !inputFocus && quickInputType == 'quickPick'` |
| P0 | `ctrl+alt+home` | `quickInput.first` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+home` | `quickInput.first` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `alt+escape` | `quickInput.hide` | `inQuickInput` |
| P0 | `ctrl+alt+escape` | `quickInput.hide` | `inQuickInput` |
| P0 | `ctrl+escape` | `quickInput.hide` | `inQuickInput` |
| P0 | `escape` | `quickInput.hide` | `inQuickInput` |
| P0 | `ctrl+alt+end` | `quickInput.last` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+end` | `quickInput.last` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+down` | `quickInput.next` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `down` | `quickInput.next` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+alt+down` | `quickInput.nextSeparator` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `alt+down` | `quickInput.nextSeparatorWithQuickAccessFallback` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `alt+pagedown` | `quickInput.pageNext` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+alt+pagedown` | `quickInput.pageNext` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+pagedown` | `quickInput.pageNext` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `pagedown` | `quickInput.pageNext` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `alt+pageup` | `quickInput.pagePrevious` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+alt+pageup` | `quickInput.pagePrevious` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+pageup` | `quickInput.pagePrevious` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `pageup` | `quickInput.pagePrevious` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+up` | `quickInput.previous` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `up` | `quickInput.previous` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `ctrl+alt+up` | `quickInput.previousSeparator` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `alt+up` | `quickInput.previousSeparatorWithQuickAccessFallback` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `space` | `quickInput.toggleCheckbox` | `inQuickInput && !inputFocus && quickInputType == 'quickPick' \|\| inQuickInput && !inputFocus && quickInputType == 'quickTree'` |
| P0 | `ctrl+space` | `quickInput.toggleHover` | `inQuickInput && quickInputType == 'quickPick' \|\| inQuickInput && quickInputType == 'quickTree'` |
| P0 | `escape` | `workbench.action.closeQuickOpen` | `inQuickOpen` |
| P0 | `shift+escape` | `workbench.action.closeQuickOpen` | `inQuickOpen` |
| P0 | `ctrl+g` | `workbench.action.gotoLine` |  |
| P0 | `ctrl+shift+o` | `workbench.action.gotoSymbol` | `!accessibilityHelpIsShown && !accessibleViewIsShown` |
| P0 | `ctrl+r` | `workbench.action.openRecent` |  |
| P0 | `alt+left` | `workbench.action.quickInputBack` | `inQuickOpen` |
| P0 | `ctrl+e` | `workbench.action.quickOpen` |  |
| P0 | `ctrl+p` | `workbench.action.quickOpen` |  |
| P0 | `ctrl+shift+tab` | `workbench.action.quickOpenLeastRecentlyUsedEditorInGroup` | `!activeEditorGroupEmpty` |
| P0 | `ctrl+tab` | `workbench.action.quickOpenNavigateNextInEditorPicker` | `inEditorsPicker && inQuickOpen` |
| P0 | `ctrl+e` | `workbench.action.quickOpenNavigateNextInFilePicker` | `inFilesPicker && inQuickOpen` |
| P0 | `ctrl+p` | `workbench.action.quickOpenNavigateNextInFilePicker` | `inFilesPicker && inQuickOpen` |
| P0 | `ctrl+r` | `workbench.action.quickOpenNavigateNextInRecentFilesPicker` | `inQuickOpen && inRecentFilesPicker` |
| P0 | `ctrl+q` | `workbench.action.quickOpenNavigateNextInViewPicker` | `inQuickOpen && inViewsPicker` |
| P0 | `ctrl+shift+tab` | `workbench.action.quickOpenNavigatePreviousInEditorPicker` | `inEditorsPicker && inQuickOpen` |
| P0 | `ctrl+shift+e` | `workbench.action.quickOpenNavigatePreviousInFilePicker` | `inFilesPicker && inQuickOpen` |
| P0 | `ctrl+shift+p` | `workbench.action.quickOpenNavigatePreviousInFilePicker` | `inFilesPicker && inQuickOpen` |
| P0 | `ctrl+shift+r` | `workbench.action.quickOpenNavigatePreviousInRecentFilesPicker` | `inQuickOpen && inRecentFilesPicker` |
| P0 | `ctrl+shift+q` | `workbench.action.quickOpenNavigatePreviousInViewPicker` | `inQuickOpen && inViewsPicker` |
| P0 | `ctrl+tab` | `workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup` | `!activeEditorGroupEmpty` |
| P0 | `ctrl+q` | `workbench.action.quickOpenView` |  |
| P0 | `ctrl+k ctrl+p` | `workbench.action.showAllEditors` |  |
| P0 | `ctrl+shift+p` | `workbench.action.showCommands` |  |
| P0 | `f1` | `workbench.action.showCommands` |  |
| P1 | `ctrl+t` | `workbench.action.showAllSymbols` |  |

### 搜索（30）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `escape` | `closeReplaceInFilesWidget` | `replaceInputBoxFocus && searchViewletVisible` |
| P0 | `escape` | `search.action.cancel` | `listFocus && searchViewletVisible && !inputFocus && !treestickyScrollFocused && searchState != '0'` |
| P0 | `ctrl+c` | `search.action.copyMatch` | `fileMatchOrMatchFocus` |
| P0 | `shift+alt+c` | `search.action.copyPath` | `fileMatchOrFolderMatchWithResourceFocus` |
| P0 | `f4` | `search.action.focusNextSearchResult` | `hasSearchResult \|\| inSearchEditor` |
| P0 | `shift+f4` | `search.action.focusPreviousSearchResult` | `hasSearchResult \|\| inSearchEditor` |
| P0 | `escape` | `search.action.focusQueryEditorWidget` | `inSearchEditor` |
| P0 | `ctrl+up` | `search.action.focusSearchFromResults` | `accessibilityModeEnabled && searchViewletVisible \|\| firstMatchFocus && searchViewletVisible` |
| P0 | `alt+enter` | `search.action.openInEditor` | `hasSearchResult && searchViewletFocus` |
| P0 | `enter` | `search.action.openResult` | `fileMatchOrMatchFocus && searchViewletVisible` |
| P0 | `ctrl+enter` | `search.action.openResultToSide` | `fileMatchOrMatchFocus && searchViewletVisible` |
| P0 | `delete` | `search.action.remove` | `fileMatchOrMatchFocus && searchViewletVisible` |
| P0 | `shift+alt+f` | `search.action.restrictSearchToFolder` | `folderMatchWithResourceFocus && searchViewletVisible` |
| P0 | `ctrl+i` | `search.action.searchWithAI` | `hasAIResultProviderKey && searchViewletFocus` |
| P0 | `ctrl+down` | `search.focus.nextInputBox` | `inSearchEditor && inputBoxFocus \|\| inputBoxFocus && searchViewletVisible` |
| P0 | `ctrl+up` | `search.focus.previousInputBox` | `inSearchEditor && inputBoxFocus \|\| inputBoxFocus && searchViewletVisible && !searchInputBoxFocus` |
| P0 | `ctrl+shift+backspace` | `search.searchEditor.action.deleteFileResults` | `inSearchEditor` |
| P0 | `ctrl+shift+f` | `workbench.action.findInFiles` | `!isSessionsWindow` |
| P0 | `ctrl+shift+h` | `workbench.action.replaceInFiles` | `!isSessionsWindow` |
| P0 | `ctrl+shift+j` | `workbench.action.search.toggleQueryDetails` | `inSearchEditor \|\| searchViewletFocus` |
| P1 | `ctrl+shift+1` | `search.action.replace` | `isEditableItem && matchFocus && replaceActive && searchViewletVisible` |
| P1 | `ctrl+alt+enter` | `search.action.replaceAll` | `replaceActive && searchViewletVisible && !findWidgetVisible` |
| P1 | `ctrl+shift+1` | `search.action.replaceAllInFile` | `fileMatchFocus && isEditableItem && replaceActive && searchViewletVisible` |
| P1 | `ctrl+shift+enter` | `search.action.replaceAllInFile` | `fileMatchFocus && isEditableItem && replaceActive && searchViewletVisible` |
| P1 | `ctrl+shift+1` | `search.action.replaceAllInFolder` | `folderMatchFocus && isEditableItem && replaceActive && searchViewletVisible` |
| P1 | `ctrl+shift+enter` | `search.action.replaceAllInFolder` | `folderMatchFocus && isEditableItem && replaceActive && searchViewletVisible` |
| P2 | `alt+oem_minus` | `decreaseSearchEditorContextLines` | `inSearchEditor` |
| P2 | `alt+oem_plus` | `increaseSearchEditorContextLines` | `inSearchEditor` |
| P2 | `ctrl+shift+r` | `rerunSearchEditorSearch` | `inSearchEditor` |
| P2 | `ctrl+shift+l` | `selectAllSearchEditorMatches` | `inSearchEditor` |

### 文件（36）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `enter` | `explorer.openAndPassFocus` | `filesExplorerFocus && foldersViewVisible && !explorerResourceIsFolder && !inputFocus` |
| P0 | `ctrl+enter` | `explorer.openToSide` | `explorerViewletFocus && foldersViewVisible && !inputFocus` |
| P0 | `home` | `firstCompressedFolder` | `explorerViewletCompressedFocus && filesExplorerFocus && foldersViewVisible && !explorerViewletCompressedFirstFocus && !inputFocus` |
| P0 | `end` | `lastCompressedFolder` | `explorerViewletCompressedFocus && filesExplorerFocus && foldersViewVisible && !explorerViewletCompressedLastFocus && !inputFocus` |
| P0 | `right` | `nextCompressedFolder` | `explorerViewletCompressedFocus && filesExplorerFocus && foldersViewVisible && !explorerViewletCompressedLastFocus && !inputFocus` |
| P0 | `left` | `previousCompressedFolder` | `explorerViewletCompressedFocus && filesExplorerFocus && foldersViewVisible && !explorerViewletCompressedFirstFocus && !inputFocus` |
| P0 | `ctrl+k p` | `workbench.action.files.copyPathOfActiveFile` |  |
| P0 | `ctrl+n` | `workbench.action.files.newUntitledFile` |  |
| P0 | `ctrl+o` | `workbench.action.files.openFile` | `true` |
| P0 | `ctrl+o` | `workbench.action.files.openFileFolder` | `isMacNative && openFolderWorkspaceSupport` |
| P0 | `ctrl+k ctrl+o` | `workbench.action.files.openFolder` | `openFolderWorkspaceSupport` |
| P0 | `ctrl+o` | `workbench.action.files.openFolderViaWorkspace` | `!openFolderWorkspaceSupport && workbenchState == 'workspace'` |
| P0 | `ctrl+k r` | `workbench.action.files.revealActiveFileInWindows` |  |
| P0 | `ctrl+s` | `workbench.action.files.save` |  |
| P0 | `ctrl+shift+s` | `workbench.action.files.saveAs` |  |
| P0 | `ctrl+shift+s` | `workbench.action.files.saveLocalFile` | `remoteFileDialogVisible` |
| P0 | `ctrl+k ctrl+shift+s` | `workbench.action.files.saveWithoutFormatting` |  |
| P0 | `ctrl+k c` | `workbench.files.action.compareWithClipboard` | `!isSessionsWindow` |
| P0 | `ctrl+k e` | `workbench.files.action.focusOpenEditorsView` | `workbench.explorer.openEditorsView.active` |
| P0 | `ctrl+shift+e` | `workbench.view.explorer` | `viewContainer.workbench.view.explorer.enabled` |
| P1 | `shift+alt+c` | `copyFilePath` | `!editorFocus` |
| P1 | `shift+alt+c` | `copyFilePath` | `editorFocus` |
| P1 | `ctrl+k ctrl+shift+c` | `copyRelativeFilePath` | `!editorFocus` |
| P1 | `ctrl+k ctrl+shift+c` | `copyRelativeFilePath` | `editorFocus` |
| P1 | `delete` | `deleteFile` | `filesExplorerFocus && foldersViewVisible && !explorerResourceMoveableToTrash && !inputFocus` |
| P1 | `shift+delete` | `deleteFile` | `filesExplorerFocus && foldersViewVisible && !inputFocus` |
| P1 | `escape` | `filesExplorer.cancelCut` | `explorerResourceCut && filesExplorerFocus && foldersViewVisible && !inputFocus` |
| P1 | `ctrl+c` | `filesExplorer.copy` | `filesExplorerFocus && foldersViewVisible && !explorerResourceIsRoot && !inputFocus` |
| P1 | `ctrl+x` | `filesExplorer.cut` | `filesExplorerFocus && foldersViewVisible && !explorerResourceIsRoot && !explorerResourceReadonly && !inputFocus` |
| P1 | `shift+alt+f` | `filesExplorer.findInFolder` | `explorerResourceIsFolder && filesExplorerFocus && foldersViewVisible && !inputFocus` |
| P1 | `space` | `filesExplorer.openFilePreserveFocus` | `filesExplorerFocus && foldersViewVisible && !explorerResourceIsFolder && !inputFocus` |
| P1 | `ctrl+v` | `filesExplorer.paste` | `filesExplorerFocus && foldersViewVisible && !explorerResourceReadonly && !inputFocus` |
| P1 | `delete` | `moveFileToTrash` | `explorerResourceMoveableToTrash && filesExplorerFocus && foldersViewVisible && !inputFocus` |
| P1 | `f2` | `renameFile` | `filesExplorerFocus && foldersViewVisible && !explorerResourceIsRoot && !explorerResourceReadonly && !inputFocus` |
| P1 | `shift+alt+r` | `revealFileInOS` | `!editorFocus` |
| P2 | `ctrl+k d` | `workbench.files.action.compareWithSaved` |  |

### 查找替换（26）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `ctrl+f` | `actions.find` | `editorFocus \|\| editorIsOpen` |
| P0 | `escape` | `closeFindWidget` | `editorFocus && findWidgetVisible` |
| P0 | `shift+escape` | `closeFindWidget` | `editorFocus && findWidgetVisible` |
| P0 | `enter` | `editor.action.nextMatchFindAction` | `editorFocus && findInputFocussed` |
| P0 | `f3` | `editor.action.nextMatchFindAction` | `editorFocus` |
| P0 | `ctrl+f3` | `editor.action.nextSelectionMatchFindAction` | `editorFocus` |
| P0 | `shift+enter` | `editor.action.previousMatchFindAction` | `editorFocus && findInputFocussed` |
| P0 | `shift+f3` | `editor.action.previousMatchFindAction` | `editorFocus` |
| P0 | `ctrl+shift+f3` | `editor.action.previousSelectionMatchFindAction` | `editorFocus` |
| P0 | `ctrl+alt+enter` | `editor.action.replaceAll` | `editorFocus && findWidgetVisible` |
| P0 | `ctrl+shift+1` | `editor.action.replaceOne` | `editorFocus && findWidgetVisible` |
| P0 | `enter` | `editor.action.replaceOne` | `editorFocus && findWidgetVisible && replaceInputFocussed` |
| P0 | `ctrl+h` | `editor.action.startFindReplaceAction` | `editorFocus \|\| editorIsOpen` |
| P0 | `alt+c` | `toggleFindCaseSensitive` | `editorFocus` |
| P0 | `alt+l` | `toggleFindInSelection` | `editorFocus` |
| P0 | `alt+r` | `toggleFindRegex` | `editorFocus` |
| P0 | `alt+w` | `toggleFindWholeWord` | `editorFocus` |
| P0 | `alt+p` | `togglePreserveCase` | `editorFocus` |
| P0 | `alt+c` | `toggleSearchCaseSensitive` | `searchViewletFocus` |
| P0 | `alt+c` | `toggleSearchEditorCaseSensitive` | `inSearchEditor && searchInputBoxFocus` |
| P0 | `alt+l` | `toggleSearchEditorContextLines` | `inSearchEditor` |
| P0 | `alt+r` | `toggleSearchEditorRegex` | `inSearchEditor && searchInputBoxFocus` |
| P0 | `alt+w` | `toggleSearchEditorWholeWord` | `inSearchEditor && searchInputBoxFocus` |
| P0 | `alt+p` | `toggleSearchPreserveCase` | `searchViewletFocus` |
| P0 | `alt+r` | `toggleSearchRegex` | `searchViewletFocus` |
| P0 | `alt+w` | `toggleSearchWholeWord` | `searchViewletFocus` |

### 欢迎页（2）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `escape` | `welcome.goBack` | `inWelcome && activeEditor == 'gettingStartedPage'` |
| P0 | `ctrl+alt+win+n` | `welcome.showNewFileEntries` | `!isSessionsWindow` |

### 窗口与设置（16）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `ctrl+k f` | `workbench.action.closeFolder` | `emptyWorkspaceSupport && !isSessionsWindow && workbenchState != 'empty'` |
| P0 | `alt+f4` | `workbench.action.closeWindow` |  |
| P0 | `ctrl+shift+w` | `workbench.action.closeWindow` |  |
| P0 | `ctrl+k ctrl+r` | `workbench.action.keybindingsReference` |  |
| P0 | `ctrl+shift+n` | `workbench.action.newWindow` |  |
| P0 | `ctrl+k ctrl+s` | `workbench.action.openGlobalKeybindings` |  |
| P0 | `ctrl+oem_comma` | `workbench.action.openSettings` |  |
| P0 | `ctrl+r` | `workbench.action.reloadWindow` | `isDevelopment` |
| P0 | `ctrl+k ctrl+t` | `workbench.action.selectTheme` |  |
| P0 | `ctrl+numpad_add` | `workbench.action.zoomIn` |  |
| P0 | `ctrl+oem_plus` | `workbench.action.zoomIn` |  |
| P0 | `ctrl+shift+oem_plus` | `workbench.action.zoomIn` |  |
| P0 | `ctrl+numpad_subtract` | `workbench.action.zoomOut` |  |
| P0 | `ctrl+oem_minus` | `workbench.action.zoomOut` |  |
| P0 | `ctrl+shift+oem_minus` | `workbench.action.zoomOut` |  |
| P0 | `ctrl+numpad0` | `workbench.action.zoomReset` |  |

### 编辑器组（65）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `shift+f10` | `editor.action.showContextMenu` | `textInputFocus` |
| P0 | `escape` | `leaveEditorMessage` | `messageVisible` |
| P0 | `ctrl+k s` | `saveAll` |  |
| P0 | `ctrl+f4` | `workbench.action.closeActiveEditor` |  |
| P0 | `ctrl+w` | `workbench.action.closeActiveEditor` |  |
| P0 | `ctrl+k ctrl+w` | `workbench.action.closeAllEditors` |  |
| P0 | `ctrl+k ctrl+shift+w` | `workbench.action.closeAllGroups` |  |
| P0 | `ctrl+k w` | `workbench.action.closeEditorsInGroup` |  |
| P0 | `ctrl+f4` | `workbench.action.closeGroup` | `activeEditorGroupEmpty && multipleEditorGroups` |
| P0 | `ctrl+w` | `workbench.action.closeGroup` | `activeEditorGroupEmpty && multipleEditorGroups` |
| P0 | `ctrl+k u` | `workbench.action.closeUnmodifiedEditors` |  |
| P0 | `ctrl+k o` | `workbench.action.copyEditorToNewWindow` | `activeEditor` |
| P0 | `ctrl+k m` | `workbench.action.editor.changeLanguageMode` | `!notebookEditorFocused` |
| P0 | `ctrl+k ctrl+up` | `workbench.action.focusAboveGroup` |  |
| P0 | `ctrl+k ctrl+down` | `workbench.action.focusBelowGroup` |  |
| P0 | `ctrl+8` | `workbench.action.focusEighthEditorGroup` |  |
| P0 | `ctrl+5` | `workbench.action.focusFifthEditorGroup` |  |
| P0 | `ctrl+1` | `workbench.action.focusFirstEditorGroup` |  |
| P0 | `ctrl+4` | `workbench.action.focusFourthEditorGroup` |  |
| P0 | `ctrl+k ctrl+left` | `workbench.action.focusLeftGroup` |  |
| P0 | `ctrl+k ctrl+right` | `workbench.action.focusRightGroup` |  |
| P0 | `ctrl+2` | `workbench.action.focusSecondEditorGroup` |  |
| P0 | `ctrl+7` | `workbench.action.focusSeventhEditorGroup` |  |
| P0 | `ctrl+6` | `workbench.action.focusSixthEditorGroup` |  |
| P0 | `ctrl+3` | `workbench.action.focusThirdEditorGroup` |  |
| P0 | `ctrl+k ctrl+shift+oem_5` | `workbench.action.joinEditorInGroup` | `sideBySideEditorActive` |
| P0 | `ctrl+k enter` | `workbench.action.keepEditor` |  |
| P0 | `alt+0` | `workbench.action.lastEditorInGroup` |  |
| P0 | `ctrl+9` | `workbench.action.lastEditorInGroup` |  |
| P0 | `ctrl+k down` | `workbench.action.moveActiveEditorGroupDown` |  |
| P0 | `ctrl+k left` | `workbench.action.moveActiveEditorGroupLeft` |  |
| P0 | `ctrl+k right` | `workbench.action.moveActiveEditorGroupRight` |  |
| P0 | `ctrl+k up` | `workbench.action.moveActiveEditorGroupUp` |  |
| P0 | `ctrl+shift+pageup` | `workbench.action.moveEditorLeftInGroup` |  |
| P0 | `ctrl+shift+pagedown` | `workbench.action.moveEditorRightInGroup` |  |
| P0 | `shift+alt+1` | `workbench.action.moveEditorToFirstGroup` |  |
| P0 | `shift+alt+9` | `workbench.action.moveEditorToLastGroup` |  |
| P0 | `ctrl+alt+right` | `workbench.action.moveEditorToNextGroup` |  |
| P0 | `ctrl+alt+left` | `workbench.action.moveEditorToPreviousGroup` |  |
| P0 | `ctrl+pagedown` | `workbench.action.nextEditor` |  |
| P0 | `ctrl+k ctrl+pagedown` | `workbench.action.nextEditorInGroup` |  |
| P0 | `alt+1` | `workbench.action.openEditorAtIndex1` |  |
| P0 | `alt+2` | `workbench.action.openEditorAtIndex2` |  |
| P0 | `alt+3` | `workbench.action.openEditorAtIndex3` |  |
| P0 | `alt+4` | `workbench.action.openEditorAtIndex4` |  |
| P0 | `alt+5` | `workbench.action.openEditorAtIndex5` |  |
| P0 | `alt+6` | `workbench.action.openEditorAtIndex6` |  |
| P0 | `alt+7` | `workbench.action.openEditorAtIndex7` |  |
| P0 | `alt+8` | `workbench.action.openEditorAtIndex8` |  |
| P0 | `alt+9` | `workbench.action.openEditorAtIndex9` |  |
| P0 | `ctrl+k shift+enter` | `workbench.action.pinEditor` | `!activeEditorIsPinned` |
| P0 | `ctrl+pageup` | `workbench.action.previousEditor` |  |
| P0 | `ctrl+k ctrl+pageup` | `workbench.action.previousEditorInGroup` |  |
| P0 | `ctrl+shift+t` | `workbench.action.reopenClosedEditor` |  |
| P0 | `ctrl+k ctrl+i` | `workbench.action.showHover` |  |
| P0 | `ctrl+oem_5` | `workbench.action.splitEditor` |  |
| P0 | `ctrl+k ctrl+oem_5` | `workbench.action.splitEditorDown` |  |
| P0 | `ctrl+k ctrl+shift+oem_5` | `workbench.action.splitEditorInGroup` | `activeEditorCanSplitInGroup` |
| P0 | `ctrl+k ctrl+oem_5` | `workbench.action.splitEditorLeft` |  |
| P0 | `ctrl+k ctrl+oem_5` | `workbench.action.splitEditorOrthogonal` |  |
| P0 | `ctrl+k ctrl+oem_5` | `workbench.action.splitEditorRight` |  |
| P0 | `ctrl+k ctrl+oem_5` | `workbench.action.splitEditorUp` |  |
| P0 | `shift+alt+0` | `workbench.action.toggleEditorGroupLayout` |  |
| P0 | `ctrl+k ctrl+m` | `workbench.action.toggleMaximizeEditorGroup` | `editorPartMaximizedEditorGroup \|\| editorPartMultipleEditorGroups` |
| P0 | `ctrl+k shift+enter` | `workbench.action.unpinEditor` | `activeEditorIsPinned` |

### 编辑基础（82）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `escape` | `cancelSelection` | `editorHasSelection && textInputFocus` |
| P0 | `shift+escape` | `cancelSelection` | `editorHasSelection && textInputFocus` |
| P0 | `ctrl+end` | `cursorBottom` | `textInputFocus` |
| P0 | `ctrl+shift+end` | `cursorBottomSelect` | `textInputFocus` |
| P0 | `ctrl+shift+alt+down` | `cursorColumnSelectDown` | `textInputFocus` |
| P0 | `shift+down` | `cursorColumnSelectDown` | `editorColumnSelection && textInputFocus` |
| P0 | `ctrl+shift+alt+left` | `cursorColumnSelectLeft` | `textInputFocus` |
| P0 | `shift+left` | `cursorColumnSelectLeft` | `editorColumnSelection && textInputFocus` |
| P0 | `ctrl+shift+alt+pagedown` | `cursorColumnSelectPageDown` | `textInputFocus` |
| P0 | `shift+pagedown` | `cursorColumnSelectPageDown` | `editorColumnSelection && textInputFocus` |
| P0 | `ctrl+shift+alt+pageup` | `cursorColumnSelectPageUp` | `textInputFocus` |
| P0 | `shift+pageup` | `cursorColumnSelectPageUp` | `editorColumnSelection && textInputFocus` |
| P0 | `ctrl+shift+alt+right` | `cursorColumnSelectRight` | `textInputFocus` |
| P0 | `shift+right` | `cursorColumnSelectRight` | `editorColumnSelection && textInputFocus` |
| P0 | `ctrl+shift+alt+up` | `cursorColumnSelectUp` | `textInputFocus` |
| P0 | `shift+up` | `cursorColumnSelectUp` | `editorColumnSelection && textInputFocus` |
| P0 | `down` | `cursorDown` | `textInputFocus` |
| P0 | `ctrl+shift+down` | `cursorDownSelect` | `textInputFocus` |
| P0 | `shift+down` | `cursorDownSelect` | `textInputFocus` |
| P0 | `end` | `cursorEnd` | `textInputFocus` |
| P0 | `shift+end` | `cursorEndSelect` | `textInputFocus` |
| P0 | `home` | `cursorHome` | `textInputFocus` |
| P0 | `shift+home` | `cursorHomeSelect` | `textInputFocus` |
| P0 | `left` | `cursorLeft` | `textInputFocus` |
| P0 | `shift+left` | `cursorLeftSelect` | `textInputFocus` |
| P0 | `pagedown` | `cursorPageDown` | `textInputFocus` |
| P0 | `shift+pagedown` | `cursorPageDownSelect` | `textInputFocus` |
| P0 | `pageup` | `cursorPageUp` | `textInputFocus` |
| P0 | `shift+pageup` | `cursorPageUpSelect` | `textInputFocus` |
| P0 | `right` | `cursorRight` | `textInputFocus` |
| P0 | `shift+right` | `cursorRightSelect` | `textInputFocus` |
| P0 | `ctrl+home` | `cursorTop` | `textInputFocus` |
| P0 | `ctrl+shift+home` | `cursorTopSelect` | `textInputFocus` |
| P0 | `ctrl+u` | `cursorUndo` | `textInputFocus` |
| P0 | `up` | `cursorUp` | `textInputFocus` |
| P0 | `ctrl+shift+up` | `cursorUpSelect` | `textInputFocus` |
| P0 | `shift+up` | `cursorUpSelect` | `textInputFocus` |
| P0 | `ctrl+right` | `cursorWordEndRight` | `textInputFocus && !accessibilityModeEnabled` |
| P0 | `ctrl+shift+right` | `cursorWordEndRightSelect` | `textInputFocus && !accessibilityModeEnabled` |
| P0 | `ctrl+left` | `cursorWordLeft` | `textInputFocus && !accessibilityModeEnabled` |
| P0 | `ctrl+shift+left` | `cursorWordLeftSelect` | `textInputFocus && !accessibilityModeEnabled` |
| P0 | `backspace` | `deleteLeft` | `textInputFocus` |
| P0 | `shift+backspace` | `deleteLeft` | `textInputFocus` |
| P0 | `delete` | `deleteRight` | `textInputFocus` |
| P0 | `ctrl+backspace` | `deleteWordLeft` | `textInputFocus && !editorReadonly` |
| P0 | `ctrl+delete` | `deleteWordRight` | `textInputFocus && !editorReadonly` |
| P0 | `escape` | `editor.action.cancelSelectionAnchor` | `editorTextFocus && selectionAnchorSet` |
| P0 | `ctrl+c` | `editor.action.clipboardCopyAction` |  |
| P0 | `ctrl+insert` | `editor.action.clipboardCopyAction` |  |
| P0 | `ctrl+x` | `editor.action.clipboardCutAction` |  |
| P0 | `shift+delete` | `editor.action.clipboardCutAction` |  |
| P0 | `ctrl+v` | `editor.action.clipboardPasteAction` |  |
| P0 | `shift+insert` | `editor.action.clipboardPasteAction` |  |
| P0 | `ctrl+oem_6` | `editor.action.indentLines` | `editorTextFocus && !editorReadonly` |
| P0 | `ctrl+enter` | `editor.action.insertLineAfter` | `editorTextFocus && !editorReadonly` |
| P0 | `ctrl+shift+enter` | `editor.action.insertLineBefore` | `editorTextFocus && !editorReadonly` |
| P0 | `ctrl+oem_4` | `editor.action.outdentLines` | `editorTextFocus && !editorReadonly` |
| P0 | `ctrl+shift+v` | `editor.action.pasteAsText` | `chatInputHasFocus` |
| P0 | `ctrl+a` | `editor.action.selectAll` |  |
| P0 | `alt+enter` | `editor.action.selectAllMatches` | `editorFocus && findWidgetVisible` |
| P0 | `ctrl+k ctrl+k` | `editor.action.selectFromAnchorToCursor` | `editorTextFocus && selectionAnchorSet` |
| P0 | `ctrl+k ctrl+b` | `editor.action.setSelectionAnchor` | `editorTextFocus` |
| P0 | `ctrl+l` | `expandLineSelection` | `textInputFocus` |
| P0 | `shift+tab` | `outdent` | `editorTextFocus && !editorReadonly && !editorTabMovesFocus` |
| P0 | `ctrl+shift+z` | `redo` |  |
| P0 | `ctrl+y` | `redo` |  |
| P0 | `escape` | `removeSecondaryCursors` | `editorHasMultipleSelections && textInputFocus` |
| P0 | `shift+escape` | `removeSecondaryCursors` | `editorHasMultipleSelections && textInputFocus` |
| P0 | `ctrl+down` | `scrollLineDown` | `textInputFocus` |
| P0 | `ctrl+up` | `scrollLineUp` | `textInputFocus` |
| P0 | `alt+pagedown` | `scrollPageDown` | `textInputFocus` |
| P0 | `alt+pageup` | `scrollPageUp` | `textInputFocus` |
| P0 | `tab` | `tab` | `editorTextFocus && !editorReadonly && !editorTabMovesFocus` |
| P0 | `ctrl+z` | `undo` |  |
| P1 | `insert` | `editor.action.toggleOvertypeInsertMode` |  |
| P1 | `escape` | `workbench.edit.chat.cancel` | `chatSessionCurrentlyEditing && inChatInput && !editorHasMultipleSelections && !editorHasSelection && !editorHoverVisible \|\| chatSessionCurrentlyEditingInput && inChatInput && !editorHasMultipleSelections && !editorHasSelection && !editorHoverVisible` |
| P2 | `escape` | `cancelLinkedEditingInput` | `LinkedEditingInputVisible && editorTextFocus` |
| P2 | `shift+escape` | `cancelLinkedEditingInput` | `LinkedEditingInputVisible && editorTextFocus` |
| P2 | `ctrl+shift+f2` | `editor.action.linkedEditing` | `editorHasRenameProvider && editorTextFocus && !editorReadonly` |
| P2 | `escape` | `editor.cancelOperation` | `cancellableOperation` |
| P2 | `ctrl+oem_period` | `editor.changeDropType` | `dropWidgetVisible` |
| P2 | `ctrl+oem_period` | `editor.changePasteType` | `pasteWidgetVisible` |

### 通用列表（35）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `alt+down` | `history.showNext` | `historyNavigationForwardsEnabled && historyNavigationWidgetFocus && !suggestWidgetVisible` |
| P0 | `down` | `history.showNext` | `historyNavigationForwardsEnabled && historyNavigationWidgetFocus && !suggestWidgetVisible` |
| P0 | `alt+up` | `history.showPrevious` | `historyNavigationBackwardsEnabled && historyNavigationWidgetFocus && !suggestWidgetVisible` |
| P0 | `up` | `history.showPrevious` | `historyNavigationBackwardsEnabled && historyNavigationWidgetFocus && !suggestWidgetVisible` |
| P0 | `escape` | `list.clear` | `listFocus && listHasSelectionOrFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `escape` | `list.closeFind` | `listFocus && treeFindOpen` |
| P0 | `left` | `list.collapse` | `listFocus && treeElementCanCollapse && !inputFocus && !treestickyScrollFocused \|\| listFocus && treeElementHasParent && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+left` | `list.collapseAll` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `right` | `list.expand` | `listFocus && treeElementCanExpand && !inputFocus && !treestickyScrollFocused \|\| listFocus && treeElementHasChild && !inputFocus && !treestickyScrollFocused` |
| P0 | `shift+down` | `list.expandSelectionDown` | `listFocus && listSupportsMultiselect && !inputFocus && !treestickyScrollFocused` |
| P0 | `shift+up` | `list.expandSelectionUp` | `listFocus && listSupportsMultiselect && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+alt+f` | `list.find` | `listFocus && listSupportsFind` |
| P0 | `f3` | `list.find` | `listFocus && listSupportsFind` |
| P0 | `ctrl+alt+f` | `list.find.replInputFocus` | `view == 'workbench.panel.repl.view'` |
| P0 | `f3` | `list.find.replInputFocus` | `view == 'workbench.panel.repl.view'` |
| P0 | `alt+down` | `list.focusAnyDown` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `alt+home` | `list.focusAnyFirst` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `alt+end` | `list.focusAnyLast` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `alt+up` | `list.focusAnyUp` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `down` | `list.focusDown` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `home` | `list.focusFirst` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `end` | `list.focusLast` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `pagedown` | `list.focusPageDown` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `pageup` | `list.focusPageUp` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `up` | `list.focusUp` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+down` | `list.scrollDown` | `listFocus && !inputFocus && !treestickyScrollFocused && listScrollAtBoundary != 'both' && listScrollAtBoundary != 'bottom'` |
| P0 | `ctrl+up` | `list.scrollUp` | `listFocus && !inputFocus && !treestickyScrollFocused && listScrollAtBoundary != 'both' && listScrollAtBoundary != 'top'` |
| P0 | `enter` | `list.select` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+a` | `list.selectAll` | `listFocus && listSupportsMultiselect && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+k ctrl+i` | `list.showHover` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `left` | `list.stickyScroll.collapse` | `treestickyScrollFocused` |
| P0 | `enter` | `list.stickyScrollselect` | `treestickyScrollFocused` |
| P0 | `space` | `list.stickyScrolltoggleExpand` | `treestickyScrollFocused` |
| P0 | `space` | `list.toggleExpand` | `listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+shift+enter` | `list.toggleSelection` | `listFocus && !inputFocus && !treestickyScrollFocused` |

### 通知（16）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `ctrl+shift+a` | `notification.acceptPrimaryAction` | `accessibilityModeEnabled && notificationFocus \|\| accessibilityModeEnabled && notificationToastsVisible` |
| P0 | `delete` | `notification.clear` | `notificationFocus` |
| P0 | `left` | `notification.collapse` | `notificationFocus` |
| P0 | `right` | `notification.expand` | `notificationFocus` |
| P0 | `enter` | `notification.toggle` | `notificationFocus` |
| P0 | `space` | `notification.toggle` | `notificationFocus` |
| P0 | `home` | `notifications.focusFirstToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `pageup` | `notifications.focusFirstToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `end` | `notifications.focusLastToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `pagedown` | `notifications.focusLastToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `down` | `notifications.focusNextToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `up` | `notifications.focusPreviousToast` | `notificationFocus && notificationToastsVisible` |
| P0 | `escape` | `notifications.hideList` | `notificationCenterVisible` |
| P0 | `escape` | `notifications.hideToasts` | `notificationToastsVisible` |
| P0 | `escape` | `notifications.hideToasts` | `notificationFocus && notificationToastsVisible` |
| P0 | `ctrl+k ctrl+shift+n` | `notifications.showList` |  |

### 阅读（42）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P0 | `ctrl+shift+oem_1` | `breadcrumbs.focus` | `breadcrumbsPossible && breadcrumbsVisible` |
| P0 | `ctrl+shift+oem_period` | `breadcrumbs.focusAndSelect` | `breadcrumbsPossible && breadcrumbsVisible` |
| P0 | `ctrl+right` | `breadcrumbs.focusNext` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `right` | `breadcrumbs.focusNext` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `ctrl+right` | `breadcrumbs.focusNextWithPicker` | `breadcrumbsActive && breadcrumbsVisible && listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+left` | `breadcrumbs.focusPrevious` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `left` | `breadcrumbs.focusPrevious` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `ctrl+left` | `breadcrumbs.focusPreviousWithPicker` | `breadcrumbsActive && breadcrumbsVisible && listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `ctrl+enter` | `breadcrumbs.revealFocused` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `space` | `breadcrumbs.revealFocused` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `ctrl+enter` | `breadcrumbs.revealFocusedFromTreeAside` | `breadcrumbsActive && breadcrumbsVisible && listFocus && !inputFocus && !treestickyScrollFocused` |
| P0 | `escape` | `breadcrumbs.selectEditor` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `down` | `breadcrumbs.selectFocused` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `enter` | `breadcrumbs.selectFocused` | `breadcrumbsActive && breadcrumbsVisible` |
| P0 | `ctrl+shift+oem_period` | `breadcrumbs.toggleToOn` | `!config.breadcrumbs.enabled` |
| P0 | `alt+z` | `editor.action.toggleWordWrap` |  |
| P0 | `ctrl+k ctrl+oem_comma` | `editor.createFoldingRangeFromSelection` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+shift+oem_4` | `editor.fold` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+0` | `editor.foldAll` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_2` | `editor.foldAllBlockComments` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_minus` | `editor.foldAllExcept` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+8` | `editor.foldAllMarkerRegions` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+1` | `editor.foldLevel1` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+2` | `editor.foldLevel2` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+3` | `editor.foldLevel3` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+4` | `editor.foldLevel4` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+5` | `editor.foldLevel5` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+6` | `editor.foldLevel6` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+7` | `editor.foldLevel7` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_4` | `editor.foldRecursively` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_period` | `editor.removeManualFoldingRanges` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+l` | `editor.toggleFold` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+shift+l` | `editor.toggleFoldRecursively` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+shift+oem_6` | `editor.unfold` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+j` | `editor.unfoldAll` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_plus` | `editor.unfoldAllExcept` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+9` | `editor.unfoldAllMarkerRegions` | `editorTextFocus && foldingEnabled` |
| P0 | `ctrl+k ctrl+oem_6` | `editor.unfoldRecursively` | `editorTextFocus && foldingEnabled` |
| P1 | `enter` | `editor.action.goToFocusedStickyScrollLine` | `stickyScrollFocused` |
| P1 | `down` | `editor.action.selectNextStickyScrollLine` | `stickyScrollFocused` |
| P1 | `up` | `editor.action.selectPreviousStickyScrollLine` | `stickyScrollFocused` |
| P1 | `shift+alt+d` | `editor.detectLanguage` | `editorTextFocus && !notebookEditable` |

### 多光标（8）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `ctrl+shift+l` | `addCursorsAtSearchResults` | `fileMatchOrMatchFocus && searchViewletVisible` |
| P1 | `ctrl+d` | `editor.action.addSelectionToNextFindMatch` | `editorFocus` |
| P1 | `ctrl+f2` | `editor.action.changeAll` | `editorTextFocus && !editorReadonly` |
| P1 | `ctrl+alt+up` | `editor.action.insertCursorAbove` | `editorTextFocus` |
| P1 | `shift+alt+i` | `editor.action.insertCursorAtEndOfEachLineSelected` | `editorTextFocus` |
| P1 | `ctrl+alt+down` | `editor.action.insertCursorBelow` | `editorTextFocus` |
| P1 | `ctrl+k ctrl+d` | `editor.action.moveSelectionToNextFindMatch` | `editorFocus` |
| P1 | `ctrl+shift+l` | `editor.action.selectHighlights` | `editorFocus` |

### 括号（2）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `ctrl+shift+oem_5` | `editor.action.jumpToBracket` | `editorTextFocus` |
| P1 | `ctrl+alt+backspace` | `editor.action.removeBrackets` | `editorTextFocus` |

### 智能扩选（2）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `shift+alt+right` | `editor.action.smartSelect.expand` | `editorTextFocus` |
| P1 | `shift+alt+left` | `editor.action.smartSelect.shrink` | `editorTextFocus` |

### 格式化（3）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `shift+alt+f` | `editor.action.formatDocument` | `editorHasDocumentFormattingProvider && editorTextFocus && !editorReadonly && !inCompositeEditor` |
| P1 | `shift+alt+f` | `editor.action.formatDocument.none` | `editorTextFocus && !editorHasDocumentFormattingProvider && !editorReadonly` |
| P1 | `ctrl+k ctrl+f` | `editor.action.formatSelection` | `editorHasDocumentSelectionFormattingProvider && editorTextFocus && !editorReadonly` |

### 注释（4）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `ctrl+k ctrl+c` | `editor.action.addCommentLine` | `editorTextFocus && !editorReadonly` |
| P1 | `shift+alt+a` | `editor.action.blockComment` | `editorTextFocus && !editorReadonly` |
| P1 | `ctrl+oem_2` | `editor.action.commentLine` | `editorTextFocus && !editorReadonly` |
| P1 | `ctrl+k ctrl+u` | `editor.action.removeCommentLine` | `editorTextFocus && !editorReadonly` |

### 行操作（6）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `shift+alt+down` | `editor.action.copyLinesDownAction` | `editorTextFocus && !editorReadonly` |
| P1 | `shift+alt+up` | `editor.action.copyLinesUpAction` | `editorTextFocus && !editorReadonly` |
| P1 | `ctrl+shift+k` | `editor.action.deleteLines` | `textInputFocus && !editorReadonly` |
| P1 | `alt+down` | `editor.action.moveLinesDownAction` | `editorTextFocus && !editorReadonly` |
| P1 | `alt+up` | `editor.action.moveLinesUpAction` | `editorTextFocus && !editorReadonly` |
| P1 | `ctrl+k ctrl+x` | `editor.action.trimTrailingWhitespace` | `editorTextFocus && !editorReadonly` |

### 设置（26）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `escape` | `settings.action.clearSearchResults` | `inSettingsEditor && inSettingsSearch` |
| P1 | `escape` | `settings.action.focusLevelUp` | `inSettingsEditor && !inSettingsJSONEditor && !inSettingsSearch` |
| P1 | `up` | `settings.action.focusSearchFromSettings` | `inSettingsEditor && settingFirstRowFocus && settingRowFocus` |
| P1 | `enter` | `settings.action.focusSettingControl` | `inSettingsEditor && settingRowFocus` |
| P1 | `down` | `settings.action.focusSettingsFromSearch` | `inSettingsSearch && !suggestWidgetVisible` |
| P1 | `enter` | `settings.action.focusSettingsFromSearchOnEnter` | `inSettingsSearch && !suggestWidgetVisible` |
| P1 | `enter` | `settings.action.focusSettingsList` | `inSettingsEditor && settingsTocRowFocus` |
| P1 | `left` | `settings.action.focusTOC` | `inSettingsEditor && settingRowFocus` |
| P1 | `ctrl+f` | `settings.action.search` | `inSettingsEditor` |
| P1 | `shift+f9` | `settings.action.showContextMenu` | `inSettingsEditor` |
| P1 | `up` | `settings.action.showPreviousSearch` | `inSettingsSearch && !suggestWidgetVisible` |
| P1 | `ctrl+i` | `settings.action.toggleAiSearch` | `aiSettingResultsAvailable && inSettingsEditor` |
| P2 | `ctrl+k ctrl+k` | `editor.action.defineKeybinding` | `resource == 'vscode-userdata:/c%3A/Users/zhaowenlong/AppData/Roaming/Code/User/keybindings.json'` |
| P2 | `escape` | `editor.action.selectEditor` | `stickyScrollFocused` |
| P2 | `enter` | `keybindings.editor.acceptWhenExpression` | `inKeybindings && whenFocus && !suggestWidgetVisible` |
| P2 | `ctrl+k ctrl+a` | `keybindings.editor.addKeybinding` | `inKeybindings && keybindingFocus` |
| P2 | `escape` | `keybindings.editor.clearSearchResults` | `inKeybindings && inKeybindingsSearch && keybindingsSearchHasValue` |
| P2 | `ctrl+c` | `keybindings.editor.copyKeybindingEntry` | `inKeybindings && keybindingFocus && !whenFocus` |
| P2 | `enter` | `keybindings.editor.defineKeybinding` | `inKeybindings && keybindingFocus && !whenFocus` |
| P2 | `ctrl+k ctrl+e` | `keybindings.editor.defineWhenExpression` | `inKeybindings && keybindingFocus` |
| P2 | `ctrl+down` | `keybindings.editor.focusKeybindings` | `inKeybindings && inKeybindingsSearch` |
| P2 | `alt+k` | `keybindings.editor.recordSearchKeys` | `inKeybindings && inKeybindingsSearch` |
| P2 | `escape` | `keybindings.editor.rejectWhenExpression` | `inKeybindings && whenFocus && !suggestWidgetVisible` |
| P2 | `delete` | `keybindings.editor.removeKeybinding` | `inKeybindings && keybindingFocus && !inputFocus` |
| P2 | `ctrl+f` | `keybindings.editor.searchKeybindings` | `inKeybindings` |
| P2 | `alt+p` | `keybindings.editor.toggleSortByPrecedence` | `inKeybindings` |

### 问题（13）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P1 | `escape` | `closeMarkersNavigation` | `editorFocus && markersNavigationVisible` |
| P1 | `shift+escape` | `closeMarkersNavigation` | `editorFocus && markersNavigationVisible` |
| P1 | `alt+f8` | `editor.action.marker.next` | `editorFocus` |
| P1 | `f8` | `editor.action.marker.nextInFiles` | `editorFocus` |
| P1 | `shift+alt+f8` | `editor.action.marker.prev` | `editorFocus` |
| P1 | `shift+f8` | `editor.action.marker.prevInFiles` | `editorFocus` |
| P1 | `escape` | `problems.action.clearFilterText` | `problemsFilterFocus` |
| P1 | `ctrl+c` | `problems.action.copy` | `problemsVisibility && !relatedInformationFocus && focusedView == 'workbench.panel.markers.view'` |
| P1 | `ctrl+f` | `problems.action.focusFilter` | `focusedView == 'workbench.panel.markers.view'` |
| P1 | `ctrl+down` | `problems.action.focusProblemsFromFilter` | `problemsFilterFocus` |
| P1 | `enter` | `problems.action.open` | `problemFocus` |
| P1 | `ctrl+enter` | `problems.action.openToSide` | `problemFocus` |
| P1 | `ctrl+oem_period` | `problems.action.showQuickFixes` | `problemFocus` |

### 对比（4）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P2 | `escape` | `diffEditor.exitCompareMove` | `comparingMovedCode` |
| P2 | `alt+f5` | `workbench.action.compareEditor.nextChange` | `textCompareEditorVisible` |
| P2 | `ctrl+k shift+o` | `workbench.action.compareEditor.openSide` | `inDiffEditor` |
| P2 | `shift+alt+f5` | `workbench.action.compareEditor.previousChange` | `textCompareEditorVisible` |

### 无障碍（25）

| 优先级 | 键 | 命令 | when |
|---|---|---|---|
| P2 | `ctrl+left` | `cursorWordAccessibilityLeft` | `accessibilityModeEnabled && isWindows && textInputFocus && focusedView == 'workbench.panel.output'` |
| P2 | `ctrl+shift+left` | `cursorWordAccessibilityLeftSelect` | `accessibilityModeEnabled && isWindows && textInputFocus && focusedView == 'workbench.panel.output'` |
| P2 | `ctrl+right` | `cursorWordAccessibilityRight` | `accessibilityModeEnabled && isWindows && textInputFocus && focusedView == 'workbench.panel.output'` |
| P2 | `ctrl+shift+right` | `cursorWordAccessibilityRightSelect` | `accessibilityModeEnabled && isWindows && textInputFocus && focusedView == 'workbench.panel.output'` |
| P2 | `alt+f1` | `editor.action.accessibilityHelp` | `!accessibilityHelpIsShown` |
| P2 | `alt+a` | `editor.action.accessibilityHelpConfigureAssignedKeybindings` | `accessibilityHelpIsShown && accessibleViewHasAssignedKeybindings` |
| P2 | `alt+k` | `editor.action.accessibilityHelpConfigureKeybindings` | `accessibilityHelpIsShown && accessibleViewHasUnassignedKeybindings` |
| P2 | `alt+h` | `editor.action.accessibilityHelpOpenHelpLink` | `accessibilityHelpIsShown` |
| P2 | `alt+f2` | `editor.action.accessibleView` |  |
| P2 | `alt+f6` | `editor.action.accessibleViewDisableHint` | `accessibilityHelpIsShown && accessibleViewVerbosityEnabled \|\| accessibleViewIsShown && accessibleViewVerbosityEnabled` |
| P2 | `ctrl+shift+o` | `editor.action.accessibleViewGoToSymbol` | `accessibilityHelpIsShown && accessibleViewGoToSymbolSupported \|\| accessibleViewGoToSymbolSupported && accessibleViewIsShown` |
| P2 | `ctrl+shift+oem_period` | `editor.action.accessibleViewGoToSymbol` | `accessibilityHelpIsShown && accessibleViewGoToSymbolSupported \|\| accessibleViewGoToSymbolSupported && accessibleViewIsShown` |
| P2 | `alt+oem_6` | `editor.action.accessibleViewNext` | `accessibleViewIsShown && accessibleViewSupportsNavigation` |
| P2 | `ctrl+alt+pagedown` | `editor.action.accessibleViewNextCodeBlock` | `accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'inlineChat' \|\| accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'panelChat' \|\| accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'quickChat'` |
| P2 | `alt+oem_4` | `editor.action.accessibleViewPrevious` | `accessibleViewIsShown && accessibleViewSupportsNavigation` |
| P2 | `ctrl+alt+pageup` | `editor.action.accessibleViewPreviousCodeBlock` | `accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'inlineChat' \|\| accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'panelChat' \|\| accessibleViewContainsCodeBlocks && accessibleViewCurrentProviderId == 'quickChat'` |
| P2 | `ctrl+shift+alt+g` | `editor.action.announceCursorPosition` | `accessibilityModeEnabled && editorTextFocus` |
| P2 | `ctrl+e` | `editor.action.toggleScreenReaderAccessibilityMode` | `accessibilityHelpIsShown` |
| P2 | `shift+alt+f1` | `editor.action.toggleScreenReaderAccessibilityMode` |  |
| P2 | `ctrl+m` | `editor.action.toggleTabFocusMode` |  |
| P2 | `backspace` | `noteMultiCursor.deleteLeft` | `config.notebook.multiCursor.enabled && isNotebookMultiSelect && activeEditor == 'workbench.editor.notebook' && notebookMultiSelectCursorState == '1' \|\| config.notebook.multiCursor.enabled && isNotebookMultiSelect && activeEditor == 'workbench.editor.notebook' && notebookMultiSelectCursorState == '2'` |
| P2 | `delete` | `noteMultiCursor.deleteRight` | `config.notebook.multiCursor.enabled && isNotebookMultiSelect && activeEditor == 'workbench.editor.notebook' && notebookMultiSelectCursorState == '1' \|\| config.notebook.multiCursor.enabled && isNotebookMultiSelect && activeEditor == 'workbench.editor.notebook' && notebookMultiSelectCursorState == '2'` |
| P2 | `escape` | `noteMultiCursor.exit` | `config.notebook.multiCursor.enabled && isNotebookMultiSelect && activeEditor == 'workbench.editor.notebook'` |
| P2 | `ctrl+down` | `widgetNavigation.focusNext` | `inputFocus && navigableContainerFocused \|\| navigableContainerFocused && treestickyScrollFocused \|\| navigableContainerFocused && !listFocus \|\| navigableContainerFocused && listScrollAtBoundary == 'both' \|\| navigableContainerFocused && listScrollAtBoundary == 'bottom'` |
| P2 | `ctrl+up` | `widgetNavigation.focusPrevious` | `inputFocus && navigableContainerFocused \|\| navigableContainerFocused && treestickyScrollFocused \|\| navigableContainerFocused && !listFocus \|\| navigableContainerFocused && listScrollAtBoundary == 'both' \|\| navigableContainerFocused && listScrollAtBoundary == 'top'` |

## 4. 不做的键位（按原因）

<details><summary>Markdown：43 个命令</summary>

`markdown.editor.copyLinesDown`, `markdown.editor.copyLinesUp`, `markdown.editor.cursorDocumentEnd`, `markdown.editor.cursorDocumentEndSelect`, `markdown.editor.cursorDocumentStart`, `markdown.editor.cursorDocumentStartSelect`, `markdown.editor.cursorDown`, `markdown.editor.cursorDownSelect`, `markdown.editor.cursorLeft`, `markdown.editor.cursorLeftSelect`, `markdown.editor.cursorLogicalLineEnd`, `markdown.editor.cursorLogicalLineEndSelect`, `markdown.editor.cursorLogicalLineStart`, `markdown.editor.cursorLogicalLineStartSelect`, `markdown.editor.cursorRight`, `markdown.editor.cursorRightSelect`, `markdown.editor.cursorUp`, `markdown.editor.cursorUpSelect`, `markdown.editor.cursorVisualLineEnd`, `markdown.editor.cursorVisualLineEndSelect`, `markdown.editor.cursorVisualLineStart`, `markdown.editor.cursorVisualLineStartSelect`, `markdown.editor.cursorWordLeft`, `markdown.editor.cursorWordLeftSelect`, `markdown.editor.cursorWordRight`, `markdown.editor.cursorWordRightSelect`, `markdown.editor.deleteLeft`, `markdown.editor.deleteLineLeft`, `markdown.editor.deleteLineRight`, `markdown.editor.deleteLines`, `markdown.editor.deleteRight`, `markdown.editor.deleteWordLeft`, `markdown.editor.deleteWordRight`, `markdown.editor.insertHardLineBreak`, `markdown.editor.insertParagraph`, `markdown.editor.joinLines`, `markdown.editor.moveLinesDown`, `markdown.editor.moveLinesUp`, `markdown.editor.redo`, `markdown.editor.selectAll`, `markdown.editor.smartEnter`, `markdown.editor.toggleLocked`, `markdown.editor.undo`

</details>

<details><summary>实时语言服务：50 个命令</summary>

`acceptAlternativeSelectedSuggestion`, `acceptRenameInput`, `acceptRenameInputWithPreview`, `acceptSelectedCodeAction`, `acceptSelectedSuggestion`, `cancelRenameInput`, `clearFilterCodeActionWidget`, `closeParameterHints`, `collapseSectionCodeAction`, `editor.action.autoFix`, `editor.action.hideColorPicker`, `editor.action.inPlaceReplace.down`, `editor.action.inPlaceReplace.up`, `editor.action.insertColorWithStandaloneColorPicker`, `editor.action.organizeImports`, `editor.action.quickFix`, `editor.action.refactor`, `editor.action.rename`, `editor.action.triggerParameterHints`, `editor.action.triggerSuggest`, `editor.emmet.action.expandAbbreviation`, `expandSectionCodeAction`, `focusNextRenameSuggestion`, `focusPreviousRenameSuggestion`, `focusSuggestion`, `hideCodeActionWidget`, `hideSuggestWidget`, `inlayHints.stopReadingLineWithHint`, `insertNextSuggestion`, `insertPrevSuggestion`, `insertSnippet`, `jumpToNextSnippetPlaceholder`, `jumpToPrevSnippetPlaceholder`, `leaveSnippet`, `markdown.editor.rename`, `markdown.editor.triggerSuggest`, `previewSelectedCodeAction`, `selectNextCodeAction`, `selectNextPageSuggestion`, `selectNextSuggestion`, `selectPrevCodeAction`, `selectPrevPageSuggestion`, `selectPrevSuggestion`, `showNextParameterHint`, `showPrevParameterHint`, `suggestWidgetCopy`, `terminalSuggestToggleExplainMode`, `toggleSectionCodeAction`, `toggleSuggestionDetails`, `toggleSuggestionFocus`

</details>

<details><summary>调试/终端/SCM/AI/扩展：384 个命令</summary>

`agentSession.archive`, `agentSession.exitAgentSessionProjection`, `agentSession.markRead`, `agentSession.markUnread`, `agentSession.rename`, `agentSession.unarchive`, `agentsVoice.cancelActiveRequest`, `agentsVoice.disconnect`, `agentsVoice.pushToTalk`, `agentsVoice.startVoiceInChat`, `agentsVoice.toggleMute`, `breakpointWidget.action.acceptInput`, `chat.action.focus`, `chat.inlineResourceAnchor.copyResource`, `chat.inlineResourceAnchor.openToSide`, `chatEditing.acceptAllFiles`, `chatEditing.discardAllFiles`, `chatEditor.action.accept`, `chatEditor.action.acceptAllEdits`, `chatEditor.action.acceptHunk`, `chatEditor.action.navigateNext`, `chatEditor.action.navigatePrevious`, `chatEditor.action.reject`, `chatEditor.action.showAccessibleDiffView`, `chatEditor.action.toggleDiff`, `chatEditor.action.undoHunk`, `closeBreakpointWidget`, `closeQuickDiff`, `commentsClearFilterText`, `commentsFocusFilter`, `commentsFocusViewFromFilter`, `debug.copyWatchExpression`, `debug.openBreakpointToSide`, `debug.openView`, `debug.removeBreakpoint`, `debug.removeWatchExpression`, `debug.renameWatchExpression`, `debug.setVariable`, `debug.toggleBreakpoint`, `editor.action.accessibleDiffViewer.next`, `editor.action.accessibleDiffViewer.prev`, `editor.action.accessibleViewAcceptInlineCompletion`, `editor.action.dirtydiff.next`, `editor.action.dirtydiff.previous`, `editor.action.extensioneditor.findNext`, `editor.action.extensioneditor.findPrevious`, `editor.action.extensioneditor.showfind`, `editor.action.inlineSuggest.acceptNextWord`, `editor.action.inlineSuggest.commit`, `editor.action.inlineSuggest.commitAlternativeAction`, `editor.action.inlineSuggest.hide`, `editor.action.inlineSuggest.jump`, `editor.action.inlineSuggest.showNext`, `editor.action.inlineSuggest.showPrevious`, `editor.action.nextCommentThreadAction`, `editor.action.nextCommentedRangeAction`, `editor.action.nextCommentingRange`, `editor.action.previousCommentThreadAction`, `editor.action.previousCommentedRangeAction`, `editor.action.previousCommentingRange`, `editor.action.submitComment`, `editor.action.webvieweditor.findNext`, `editor.action.webvieweditor.findPrevious`, `editor.action.webvieweditor.hideFind`, `editor.action.webvieweditor.showFind`, `editor.closeTestPeek`, `editor.debug.action.closeExceptionWidget`, `editor.debug.action.showDebugHover`, `editor.debug.action.toggleBreakpoint`, `editor.debug.action.toggleInlineBreakpoint`, `extension.node-debug.startWithStopOnEntry`, `inlineChat.askInChat`, `inlineChat.dismissEditorAffordance`, `inlineChat.focus`, `inlineChat.start`, `inlineChat2.cancel`, `inlineChat2.close`, `inlineChat2.keep`, `insertBestCompletion`, `interactive.execute`, `interactive.history.focus`, `interactive.history.next`, `interactive.history.previous`, `interactive.scrollToBottom`, `interactive.scrollToTop`, `mergeEditor.acceptMerge`, `mergeEditor.toggleBetweenInputs`, `multiDiffEditor.goToNextChange`, `multiDiffEditor.goToPreviousChange`, `notebook.addFindMatchToSelection`, `notebook.cell.changeLanguage`, `notebook.cell.changeToCode`, `notebook.cell.changeToMarkdown`, `notebook.cell.chat.start`, `notebook.cell.clearOutputs`, `notebook.cell.collapseCellInput`, `notebook.cell.collapseCellOutput`, `notebook.cell.copyDown`, `notebook.cell.copyUp`, `notebook.cell.cursorPageDown`, `notebook.cell.cursorPageDownSelect`, `notebook.cell.cursorPageUp`, `notebook.cell.cursorPageUpSelect`, `notebook.cell.delete`, `notebook.cell.detectLanguage`, `notebook.cell.edit`, `notebook.cell.execute`, `notebook.cell.executeAndInsertBelow`, `notebook.cell.executeAndSelectBelow`, `notebook.cell.expandCellInput`, `notebook.cell.expandCellOutput`, `notebook.cell.focusInOutput`, `notebook.cell.focusOutOutput`, `notebook.cell.insertCodeCellAbove`, `notebook.cell.insertCodeCellBelow`, `notebook.cell.joinAbove`, `notebook.cell.joinBelow`, `notebook.cell.moveDown`, `notebook.cell.moveUp`, `notebook.cell.nullAction`, `notebook.cell.openFailureActions`, `notebook.cell.output.selectAll`, `notebook.cell.pasteAbove`, `notebook.cell.quitEdit`, `notebook.cell.split`, `notebook.cell.toggleOutputScrolling`, `notebook.centerActiveCell`, `notebook.commentSelectedCells`, `notebook.diff.action.next`, `notebook.diff.action.previous`, `notebook.find`, `notebook.findNext.fromWidget`, `notebook.findPrevious.fromWidget`, `notebook.focusBottom`, `notebook.focusNextEditor`, `notebook.focusPreviousEditor`, `notebook.focusTop`, `notebook.fold`, `notebook.format`, `notebook.formatCell`, `notebook.hideFind`, `notebook.selectAllFindMatches`, `notebook.unfold`, `o.showOutput`, `refactorPreview.apply`, `refactorPreview.toggleCheckedState`, `remote.tunnel.closeInline`, `remote.tunnel.copyAddressInline`, `remote.tunnel.label`, `repl.action.acceptInput`, `repl.action.filter`, `repl.action.find`, `repl.execute`, `repl.focusLastItemExecuted`, `repl.input.focus`, `scm.acceptInput`, `scm.clearInput`, `scm.clearValidation`, `scm.forceViewNextCommit`, `scm.forceViewPreviousCommit`, `scm.viewNextCommit`, `scm.viewPreviousCommit`, `testing.cancelRun`, `testing.coverage.goToNextMissedLine`, `testing.coverage.goToPreviousMissedLine`, `testing.coverageAll`, `testing.coverageAtCursor`, `testing.coverageCurrentFile`, `testing.coverageLastRun`, `testing.debugAll`, `testing.debugAtCursor`, `testing.debugCurrentFile`, `testing.debugFailTests`, `testing.debugLastRun`, `testing.editFocusedTest`, `testing.goToNextMessage`, `testing.goToPreviousMessage`, `testing.openOutputPeek`, `testing.reRunFailTests`, `testing.reRunLastRun`, `testing.refreshTests`, `testing.runAll`, `testing.runAtCursor`, `testing.runCurrentFile`, `testing.showMostRecentOutput`, `testing.toggleInlineCoverage`, `testing.toggleInlineTestOutput`, `testing.toggleTestingPeekHistory`, `toggleExplainMode`, `toggleQuickDiffWidgetFocus`, `workbench.action.addComment`, `workbench.action.browser.addElementCommentToChat`, `workbench.action.browser.addElementToChat`, `workbench.action.browser.findNext`, `workbench.action.browser.findPrevious`, `workbench.action.browser.focusUrlInput`, `workbench.action.browser.goBack`, `workbench.action.browser.goForward`, `workbench.action.browser.hardReload`, `workbench.action.browser.hideFind`, `workbench.action.browser.newTab`, `workbench.action.browser.openOrList`, `workbench.action.browser.quickOpen`, `workbench.action.browser.reload`, `workbench.action.browser.resetZoom`, `workbench.action.browser.showFind`, `workbench.action.browser.showHistory`, `workbench.action.browser.stopElementSelection`, `workbench.action.browser.toggleDevTools`, `workbench.action.browser.toggleFavorite`, `workbench.action.browser.zoomIn`, `workbench.action.browser.zoomOut`, `workbench.action.chat.acceptElicitation`, `workbench.action.chat.acceptTool`, `workbench.action.chat.applyInEditor`, `workbench.action.chat.attachContext`, `workbench.action.chat.cancel`, `workbench.action.chat.cancelSpeechToText`, `workbench.action.chat.editRequests`, `workbench.action.chat.find`, `workbench.action.chat.findNext`, `workbench.action.chat.findPrevious`, `workbench.action.chat.focusConfirmation`, `workbench.action.chat.focusInput`, `workbench.action.chat.focusQuestionCarousel`, `workbench.action.chat.focusQuestionCarouselTerminal`, `workbench.action.chat.focusTip`, `workbench.action.chat.focusTodosView`, `workbench.action.chat.hideFind`, `workbench.action.chat.holdToVoiceChatInChatView`, `workbench.action.chat.insertCodeBlock`, `workbench.action.chat.newChat`, `workbench.action.chat.nextCodeBlock`, `workbench.action.chat.nextFileTree`, `workbench.action.chat.nextQuestion`, `workbench.action.chat.nextUserPrompt`, `workbench.action.chat.open`, `workbench.action.chat.openInlineAttachment`, `workbench.action.chat.openModePicker`, `workbench.action.chat.openModelPicker`, `workbench.action.chat.openSessionInEditorGroup`, `workbench.action.chat.openSessionInNewEditorGroup`, `workbench.action.chat.openagent`, `workbench.action.chat.previousCodeBlock`, `workbench.action.chat.previousFileTree`, `workbench.action.chat.previousQuestion`, `workbench.action.chat.previousUserPrompt`, `workbench.action.chat.queueMessage`, `workbench.action.chat.restoreCheckpoint`, `workbench.action.chat.run-in-new-chat.prompt.current`, `workbench.action.chat.run.prompt`, `workbench.action.chat.run.prompt.current`, `workbench.action.chat.runInTerminal`, `workbench.action.chat.sendToNewChat`, `workbench.action.chat.showUpdateRequired`, `workbench.action.chat.skipTool`, `workbench.action.chat.startVoiceChat`, `workbench.action.chat.steerWithMessage`, `workbench.action.chat.stopListening`, `workbench.action.chat.stopListeningAndSubmit`, `workbench.action.chat.stopReadChatItemAloud`, `workbench.action.chat.submit`, `workbench.action.chat.submitWithCodebase`, `workbench.action.chat.submitWithoutDispatching`, `workbench.action.chat.toggleFindCaseSensitive`, `workbench.action.chat.toggleFindRegex`, `workbench.action.chat.toggleFindWholeWord`, `workbench.action.chat.toggleSpeechToText`, `workbench.action.chat.toggleThinkingContentAccessibleView`, `workbench.action.chat.undoEdits`, `workbench.action.chat.voiceInputMode.holdToTalk`, `workbench.action.closeModalEditor`, `workbench.action.debug.continue`, `workbench.action.debug.disconnect`, `workbench.action.debug.nextConsole`, `workbench.action.debug.pause`, `workbench.action.debug.prevConsole`, `workbench.action.debug.restart`, `workbench.action.debug.run`, `workbench.action.debug.start`, `workbench.action.debug.stepInto`, `workbench.action.debug.stepIntoTarget`, `workbench.action.debug.stepOut`, `workbench.action.debug.stepOver`, `workbench.action.debug.stop`, `workbench.action.editorDictation.start`, `workbench.action.files.openLocalFile`, `workbench.action.files.openLocalFolder`, `workbench.action.hideComment`, `workbench.action.interactivePlayground.arrowDown`, `workbench.action.interactivePlayground.arrowUp`, `workbench.action.interactivePlayground.pageDown`, `workbench.action.interactivePlayground.pageUp`, `workbench.action.issueReporter.captureScreenshot`, `workbench.action.issueReporter.toggleRecording`, `workbench.action.navigateModalEditorNext`, `workbench.action.navigateModalEditorPrevious`, `workbench.action.openAgentsWindow`, `workbench.action.openChat`, `workbench.action.output.toggleOutput`, `workbench.action.quickchat.toggle`, `workbench.action.remote.showMenu`, `workbench.action.speech.stopReadAloud`, `workbench.action.tasks.build`, `workbench.action.tasks.rerunForActiveTerminal`, `workbench.action.terminal.acceptSelectedSuggestion`, `workbench.action.terminal.acceptSelectedSuggestionEnter`, `workbench.action.terminal.accessibleBufferGoToNextCommand`, `workbench.action.terminal.accessibleBufferGoToPreviousCommand`, `workbench.action.terminal.chat.close`, `workbench.action.terminal.chat.focusMostRecentChatTerminalOutput`, `workbench.action.terminal.chat.insertCommand`, `workbench.action.terminal.chat.insertFirstCommand`, `workbench.action.terminal.chat.rerunRequest`, `workbench.action.terminal.chat.runCommand`, `workbench.action.terminal.chat.runFirstCommand`, `workbench.action.terminal.chat.start`, `workbench.action.terminal.clearSelection`, `workbench.action.terminal.copyAndClearSelection`, `workbench.action.terminal.copySelection`, `workbench.action.terminal.findNext`, `workbench.action.terminal.findPrevious`, `workbench.action.terminal.focus`, `workbench.action.terminal.focusAccessibleBuffer`, `workbench.action.terminal.focusFind`, `workbench.action.terminal.focusHover`, `workbench.action.terminal.focusNext`, `workbench.action.terminal.focusNextPane`, `workbench.action.terminal.focusPrevious`, `workbench.action.terminal.focusPreviousPane`, `workbench.action.terminal.focusTabs`, `workbench.action.terminal.goToRecentDirectory`, `workbench.action.terminal.hideFind`, `workbench.action.terminal.hideSuggestWidget`, `workbench.action.terminal.hideSuggestWidgetAndNavigateHistory`, `workbench.action.terminal.killActiveTab`, `workbench.action.terminal.killEditor`, `workbench.action.terminal.new`, `workbench.action.terminal.newInNewWindow`, `workbench.action.terminal.openDetectedLink`, `workbench.action.terminal.openNativeConsole`, `workbench.action.terminal.paste`, `workbench.action.terminal.pastePwsh`, `workbench.action.terminal.renameActiveTab`, `workbench.action.terminal.runRecentCommand`, `workbench.action.terminal.scrollDown`, `workbench.action.terminal.scrollDownPage`, `workbench.action.terminal.scrollToBottom`, `workbench.action.terminal.scrollToBottomAccessibleView`, `workbench.action.terminal.scrollToNextCommand`, `workbench.action.terminal.scrollToPreviousCommand`, `workbench.action.terminal.scrollToTop`, `workbench.action.terminal.scrollToTopAccessibleView`, `workbench.action.terminal.scrollUp`, `workbench.action.terminal.scrollUpPage`, `workbench.action.terminal.searchWorkspace`, `workbench.action.terminal.selectNextPageSuggestion`, `workbench.action.terminal.selectNextSuggestion`, `workbench.action.terminal.selectPrevPageSuggestion`, `workbench.action.terminal.selectPrevSuggestion`, `workbench.action.terminal.selectToNextCommand`, `workbench.action.terminal.selectToPreviousCommand`, `workbench.action.terminal.sendSequence`, `workbench.action.terminal.showQuickFixes`, `workbench.action.terminal.sizeToContentWidth`, `workbench.action.terminal.split`, `workbench.action.terminal.splitActiveTab`, `workbench.action.terminal.suggestToggleDetails`, `workbench.action.terminal.suggestToggleDetailsFocus`, `workbench.action.terminal.toggleFindCaseSensitive`, `workbench.action.terminal.toggleFindRegex`, `workbench.action.terminal.toggleFindWholeWord`, `workbench.action.terminal.toggleTerminal`, `workbench.action.terminal.triggerSuggest`, `workbench.action.toggleDevTools`, `workbench.banner.focusBanner`, `workbench.banner.focusNextAction`, `workbench.banner.focusPreviousAction`, `workbench.chat.action.focusLastFocused`, `workbench.debug.action.toggleRepl`, `workbench.debug.viewlet.action.copyValue`, `workbench.view.debug`, `workbench.view.extensions`, `workbench.view.scm`

</details>

## 5. Rove 专有命令

命令 ID 以 `rove.` 开头。默认键位已与 VS Code 的全部默认键位逐一核对，没有冲突；还特意避开了 `ctrl+alt+a`（国内常见的截图全局热键）。

| 优先级 | 命令 | 默认键 | when | 说明 | 冲突检查 |
|---|---|---|---|---|---|
| P1 | `rove.contextBasket.add` | `ctrl+k b` | `editorTextFocus` | 把选区（没有选区时为光标所在符号）加入上下文篮子（UX_SPEC §5.1） | 无冲突 |
| P1 | `rove.contextBasket.focus` | `ctrl+shift+alt+b` |  | 打开上下文篮子视图 | 无冲突 |
| P1 | `rove.contextBasket.copyAsMarkdown` | — |  | 复制为 Markdown（UX_SPEC §5.3） |  |
| P1 | `rove.contextBasket.copyAsReferences` | — |  | 复制为引用（UX_SPEC §5.3） |  |
| P1 | `rove.contextBasket.clear` | — |  | 清空上下文篮子 |  |
| P1 | `rove.editor.followTail` | — |  | 跟随文件末尾（UX_SPEC §3.2） |  |
| P1 | `rove.editor.viewFormatted` | — |  | 格式化后查看（只读副本，UX_SPEC §3.3） |  |
| P0 | `rove.editor.openLongLine` | — |  | 在临时标签中查看被截断的超长行（UX_SPEC §3.3） |  |
| P1 | `rove.json.minify` | — |  | 压缩 JSON（UX_SPEC §4.2） |  |
| P1 | `rove.structure.copyPath` | — |  | 复制结构路径（JSON/YAML 为 `$.a[3]`，XML 为 XPath，UX_SPEC §4.2） |  |
| P1 | `rove.structure.copyValue` | — |  | 复制节点的值（UX_SPEC §4.2） |  |
| P0 | `rove.index.rebuild` | — |  | 重建 C# 语义索引（UX_SPEC §2.1） |  |
| P0 | `rove.index.showDetails` | — |  | 显示 C# 索引详情（UX_SPEC §2.1） |  |
| P0 | `rove.index.openLog` | — |  | 打开索引日志（UX_SPEC §2.1） |  |

## 6. 需要支持的 `when` 上下文键

以下是“需要实现的键位”中引用到的上下文键（按引用次数排序）。Rove 必须实现这些上下文键；不认识的上下文键一律视为 false（FEATURES §13 规则 7）。

`editorTextFocus`（72）, `inQuickInput`（64）, `quickInputType`（60）, `textInputFocus`（59）, `inputFocus`（58）, `listFocus`（36）, `treestickyScrollFocused`（36）, `editorFocus`（35）, `quickPick`（30）, `editorReadonly`（23）, `foldingEnabled`（22）, `quickTree`（22）, `searchViewletVisible`（17）, `foldersViewVisible`（16）, `inSearchEditor`（15）, `filesExplorerFocus`（15）, `breadcrumbsVisible`（14）, `notificationFocus`（13）, `inQuickOpen`（13）, `accessibilityModeEnabled`（12）, `editorHoverFocused`（12）, `breadcrumbsActive`（12）, `inKeybindings`（12）, `inReferenceSearchEditor`（11）, `referenceSearchVisible`（11）, `focusedView`（10）, `navigableContainerFocused`（10）, `inSettingsEditor`（9）, `notificationToastsVisible`（9）, `accessibilityHelpIsShown`（9）, `suggestWidgetVisible`（9）, `listScrollAtBoundary`（8）, `quickWidget`（8）, `activeEditor`（7）, `findWidgetVisible`（7）, `searchViewletFocus`（7）, `statusBarFocused`（7）, `editorColumnSelection`（6）, `accessibleViewIsShown`（6）, `accessibleViewContainsCodeBlocks`（6）, `accessibleViewCurrentProviderId`（6）, `replaceActive`（6）, `isInEmbeddedEditor`（5）, `editorHasDefinitionProvider`（5）, `inSettingsSearch`（5）, `fileMatchOrMatchFocus`（5）, `keybindingFocus`（5）, `isNotebookMultiSelect`（5）, `workbench.editor.notebook`（5）, `isEditableItem`（5）, `isSessionsWindow`（5）, `editorHasSelection`（4）, `editorHasMultipleSelections`（4）, `stickyScrollFocused`（4）, `isWindows`（4）, `workbench.panel.output`（4）, `callHierarchyVisible`（4）, `typeHierarchyVisible`（4）, `historyNavigationWidgetFocus`（4）, `whenFocus`（4）, `both`（4）, `notebookMultiSelectCursorState`（4）, `inputBoxFocus`（4）, `searchInputBoxFocus`（4）, `activeEditorGroupEmpty`（4）, `accessibleViewGoToSymbolSupported`（4）, `explorerViewletCompressedFocus`（4）, `cursorAtEndOfQuickInputBox`（4）, `inFilesPicker`（4）, `treeElementCanCollapse`（3）, `treeElementCanExpand`（3）, `explorerResourceIsFolder`（3）, `inKeybindingsSearch`（3）, `listSupportsMultiselect`（3）, `problemFocus`（3）, `hasSearchResult`（3）, `settingRowFocus`（3）, `notebookEditorFocused`（3）, `openFolderWorkspaceSupport`（3）, `explorerResourceReadonly`（3）, `explorerResourceIsRoot`（3）, `editorTabMovesFocus`（2）, `inChatInput`（2）, `editorHoverVisible`（2）, `editorIsOpen`（2）, `selectionAnchorSet`（2）, `editorHasDocumentFormattingProvider`（2）, `editorHasImplementationProvider`（2）, `editorHasReferenceProvider`（2）, `findInputFocussed`（2）, `isWeb`（2）, `hasWordHighlights`（2）, `dropWidgetVisible`（2）, `pasteWidgetVisible`（2）, `hasSymbols`（2）, `textCompareEditorActive`（2）, `quickDiffDecorationCount`（2）, `markersNavigationVisible`（2）, `LinkedEditingInputVisible`（2）, `breadcrumbsPossible`（2）, `accessibleViewVerbosityEnabled`（2）, `accessibleViewSupportsNavigation`（2）, `inlineChat`（2）, `panelChat`（2）, `quickChat`（2）, `editorHasCallHierarchyProvider`（2）, `callHierarchyDirection`（2）, `typeHierarchyDirection`（2）, `historyNavigationForwardsEnabled`（2）, `historyNavigationBackwardsEnabled`（2）, `listSupportsFind`（2）, `bottom`（2）, `top`（2）, `problemsFilterFocus`（2）, `workbench.panel.markers.view`（2）, `fileMatchFocus`（2）, `folderMatchFocus`（2）, `workbenchState`（2）, `multipleEditorGroups`（2）, `textCompareEditorVisible`（2）, `canNavigateBack`（2）, `canNavigateForward`（2）, `activeEditorIsPinned`（2）, `auxiliaryBarMaximized`（2）, `view`（2）, `workbench.panel.repl.view`（2）, `explorerResourceMoveableToTrash`（2）, `explorerViewletCompressedFirstFocus`（2）, `explorerViewletCompressedLastFocus`（2）, `inEditorsPicker`（2）, `inRecentFilesPicker`（2）, `inViewsPicker`（2）, `reference-list.hasResult`（2）, `references-view.canNavigate`（2）, `markdown`（2）, `prompt`（2）, `instructions`（2）, `chatagent`（2）, `skill`（2）, `terminalFocus`（2）, `inZenMode`（1）, `chatSessionCurrentlyEditing`（1）, `chatSessionCurrentlyEditingInput`（1）, `inCompositeEditor`（1）, `editorHasDocumentSelectionFormattingProvider`（1）, `editorHasRenameProvider`（1）, `cancellableOperation`（1）, `aiSettingResultsAvailable`（1）, `inWelcome`（1）, `gettingStartedPage`（1）, `replaceInputFocussed`（1）, `messageVisible`（1）, `replaceInputBoxFocus`（1）, `accessibleViewHasAssignedKeybindings`（1）, `accessibleViewHasUnassignedKeybindings`（1）, `resource`（1）, `vscode-userdata`（1）, `c`（1）, `A`（1）, `Users`（1）, `zhaowenlong`（1）, `AppData`（1）, `Roaming`（1）, `Code`（1）, `User`（1）, `keybindings.json`（1）, `chatInputHasFocus`（1）, `notebookEditable`（1）, `outgoingCalls`（1）, `incomingCalls`（1）, `supertypes`（1）, `subtypes`（1）, `explorerViewletFocus`（1）, `keybindingsSearchHasValue`（1）, `listHasSelectionOrFocus`（1）, `treeFindOpen`（1）, `treeElementHasParent`（1）, `treeElementHasChild`（1）, `problemsVisibility`（1）, `relatedInformationFocus`（1）, `searchState`（1）, `fileMatchOrFolderMatchWithResourceFocus`（1）, `firstMatchFocus`（1）, `matchFocus`（1）, `folderMatchWithResourceFocus`（1）, `hasAIResultProviderKey`（1）, `inSettingsJSONEditor`（1）, `settingsTocRowFocus`（1）, `emptyWorkspaceSupport`（1）, `empty`（1）, `inDiffEditor`（1）, `isMacNative`（1）, `workspace`（1）, `remoteFileDialogVisible`（1）, `sideBySideEditorActive`（1）, `activeEditorCanSplitInGroup`（1）, `isIOS`（1）, `editorPartMaximizedEditorGroup`（1）, `editorPartMultipleEditorGroups`（1）, `isAuxiliaryWindowFocusedContext`（1）, `workbench.panel.markers.view.active`（1）, `workbench.explorer.openEditorsView.active`（1）, `workbench.panel.chat.view.copilot.active`（1）, `viewContainer.workbench.view.explorer.enabled`（1）, `workbench.view.search.active`（1）, `neverMatch`（1）, `doesNotMatch`（1）, `settingFirstRowFocus`（1）, `explorerResourceCut`（1）, `notificationCenterVisible`（1）, `isDevelopment`（1）, `editorLangId`（1）, `activeCustomEditorId`（1）, `vscode.markdown.preview.editor`（1）, `resourceLangId`（1）, `comparingMovedCode`（1）
