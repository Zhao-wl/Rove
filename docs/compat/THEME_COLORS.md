# 主题颜色键兼容清单（阶段 0.3c-1）

> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code 官方文档的主题颜色参考（vscode-docs，批准日期 11/4/2026） · 生成脚本：`scripts/compat/gen_theme_doc.py`

## 1. 说明

- Rove 直接加载 VS Code 的颜色主题 JSON（FEATURES §7）。主题中 `colors` 部分的颜色键，按下表决定 Rove 是否使用；不使用的键静默忽略。
- 主题没有提供某个键时，与 VS Code 一样回退到该键的默认值（深色、浅色、高对比度各一套）。默认值取自 VS Code 源码中 `registerColor` 的注册，需要在实现时整理成数据表（不在本清单中）。
- 优先级跟随对应功能的优先级，例如缩略图是 P1，`minimap.*` 就是 P1。
- 对判定有异议时，修改生成脚本中的规则后重新生成。

## 2. 统计

| 判定 | 颜色键数 |
|---|---|
| 支持 P0 | 404 |
| 支持 P1 | 100 |
| 支持 P2 | 30 |
| 不支持（忽略） | 437 |
| **合计** | 971 |

## 3. 支持（P0）

**Action Bar colors**（1）：`actionBar.toggledBackground`

**Action colors**（3）：`toolbar.hoverBackground`, `toolbar.hoverOutline`, `toolbar.activeBackground`

**Activity Bar**（20）：`activityBar.background`, `activityBar.dropBorder`, `activityBar.foreground`, `activityBar.inactiveForeground`, `activityBar.border`, `activityBarBadge.background`, `activityBarBadge.foreground`, `activityBar.activeBorder`, `activityBar.activeBackground`, `activityBar.activeFocusBorder`, `activityBarTop.foreground`, `activityBarTop.activeBorder`, `activityBarTop.inactiveForeground`, `activityBarTop.dropBorder`, `activityBarTop.background`, `activityBarTop.activeBackground`, `activityWarningBadge.foreground`, `activityWarningBadge.background`, `activityErrorBadge.foreground`, `activityErrorBadge.background`

**Badge**（2）：`badge.foreground`, `badge.background`

**Base colors**（10）：`focusBorder`, `foreground`, `disabledForeground`, `widget.border`, `widget.shadow`, `selection.background`, `descriptionForeground`, `errorForeground`, `icon.foreground`, `sash.hoverBorder`

**Breadcrumbs colors**（5）：`breadcrumb.foreground`, `breadcrumb.background`, `breadcrumb.focusForeground`, `breadcrumb.activeSelectionForeground`, `breadcrumbPicker.background`

**Button control**（23）：`button.background`, `button.foreground`, `button.border`, `button.separator`, `button.hoverBackground`, `button.secondaryForeground`, `button.secondaryBackground`, `button.secondaryHoverBackground`, `button.secondaryBorder`, `checkbox.background`, `checkbox.foreground`, `checkbox.disabled.background`, `checkbox.disabled.foreground`, `checkbox.border`, `checkbox.selectBackground`, `checkbox.selectBorder`, `radio.activeForeground`, `radio.activeBackground`, `radio.activeBorder`, `radio.inactiveForeground`, `radio.inactiveBackground`, `radio.inactiveBorder`, `radio.inactiveHoverBackground`

**Command Center colors**（8）：`commandCenter.foreground`, `commandCenter.activeForeground`, `commandCenter.background`, `commandCenter.activeBackground`, `commandCenter.border`, `commandCenter.inactiveForeground`, `commandCenter.inactiveBorder`, `commandCenter.activeBorder`

**Contrast colors**（2）：`contrastActiveBorder`, `contrastBorder`

**Dropdown control**（4）：`dropdown.background`, `dropdown.listBackground`, `dropdown.border`, `dropdown.foreground`

**Editor Groups & Tabs**（42）：`editorGroup.border`, `editorGroup.dropBackground`, `editorGroupHeader.noTabsBackground`, `editorGroupHeader.tabsBackground`, `editorGroupHeader.tabsBorder`, `editorGroupHeader.border`, `editorGroup.emptyBackground`, `editorGroup.focusedEmptyBorder`, `editorGroup.dropIntoPromptForeground`, `editorGroup.dropIntoPromptBackground`, `editorGroup.dropIntoPromptBorder`, `tab.activeBackground`, `tab.unfocusedActiveBackground`, `tab.activeForeground`, `tab.border`, `tab.activeBorder`, `tab.selectedBorderTop`, `tab.selectedBackground`, `tab.selectedForeground`, `tab.dragAndDropBorder`, `tab.unfocusedActiveBorder`, `tab.activeBorderTop`, `tab.unfocusedActiveBorderTop`, `tab.lastPinnedBorder`, `tab.inactiveBackground`, `tab.unfocusedInactiveBackground`, `tab.inactiveForeground`, `tab.unfocusedActiveForeground`, `tab.unfocusedInactiveForeground`, `tab.hoverBackground`, `tab.unfocusedHoverBackground`, `tab.hoverForeground`, `tab.unfocusedHoverForeground`, `tab.hoverBorder`, `tab.unfocusedHoverBorder`, `tab.activeModifiedBorder`, `tab.inactiveModifiedBorder`, `tab.unfocusedActiveModifiedBorder`, `tab.unfocusedInactiveModifiedBorder`, `editorPane.background`, `sideBySideEditor.horizontalBorder`, `sideBySideEditor.verticalBorder`

**Editor colors**（57）：`editor.background`, `editor.foreground`, `editorLineNumber.foreground`, `editorLineNumber.activeForeground`, `editorLineNumber.dimmedForeground`, `editorCursor.background`, `editorCursor.foreground`, `editor.compositionBorder`, `editor.selectionBackground`, `editor.selectionForeground`, `editor.inactiveSelectionBackground`, `editor.selectionHighlightBackground`, `editor.selectionHighlightBorder`, `editor.findMatchBackground`, `editor.findMatchForeground`, `editor.findMatchHighlightForeground`, `editor.findMatchHighlightBackground`, `editor.findRangeHighlightBackground`, `editor.findMatchBorder`, `editor.findMatchHighlightBorder`, `editor.findRangeHighlightBorder`, `search.resultsInfoForeground`, `editor.hoverHighlightBackground`, `editor.lineHighlightBackground`, `editor.inactiveLineHighlightBackground`, `editor.lineHighlightBorder`, `editor.rangeHighlightBackground`, `editor.rangeHighlightBorder`, `editor.symbolHighlightBackground`, `editor.symbolHighlightBorder`, `editorWhitespace.foreground`, `editorIndentGuide.background`, `editorIndentGuide.background1`, `editorIndentGuide.background2`, `editorIndentGuide.background3`, `editorIndentGuide.background4`, `editorIndentGuide.background5`, `editorIndentGuide.background6`, `editorIndentGuide.activeBackground`, `editorIndentGuide.activeBackground1`, `editorIndentGuide.activeBackground2`, `editorIndentGuide.activeBackground3`, `editorIndentGuide.activeBackground4`, `editorIndentGuide.activeBackground5`, `editorIndentGuide.activeBackground6`, `editorBracketMatch.background`, `editorBracketMatch.border`, `editorBracketMatch.foreground`, `editor.foldBackground`, `editor.foldPlaceholderForeground`, `problemsErrorIcon.foreground`, `problemsWarningIcon.foreground`, `problemsInfoIcon.foreground`, `editorUnnecessaryCode.border`, `editorGutter.background`, `editorGutter.foldingControlForeground`, `editorGutter.commentDraftGlyphForeground`

**Editor widget colors**（8）：`editorWidget.foreground`, `editorWidget.background`, `editorWidget.border`, `editorWidget.resizeBorder`, `editorHoverWidget.foreground`, `editorHoverWidget.background`, `editorHoverWidget.border`, `editorHoverWidget.statusBarBackground`

**Input control**（17）：`input.background`, `input.border`, `input.foreground`, `input.placeholderForeground`, `inputOption.activeBackground`, `inputOption.activeBorder`, `inputOption.activeForeground`, `inputOption.hoverBackground`, `inputValidation.errorBackground`, `inputValidation.errorForeground`, `inputValidation.errorBorder`, `inputValidation.infoBackground`, `inputValidation.infoForeground`, `inputValidation.infoBorder`, `inputValidation.warningBackground`, `inputValidation.warningForeground`, `inputValidation.warningBorder`

**Keybinding label colors**（4）：`keybindingLabel.background`, `keybindingLabel.foreground`, `keybindingLabel.border`, `keybindingLabel.bottomBorder`

**Lists and trees**（32）：`list.activeSelectionBackground`, `list.activeSelectionForeground`, `list.activeSelectionIconForeground`, `list.dropBackground`, `list.focusBackground`, `list.focusForeground`, `list.focusHighlightForeground`, `list.focusOutline`, `list.focusAndSelectionOutline`, `list.highlightForeground`, `list.hoverBackground`, `list.hoverForeground`, `list.inactiveSelectionBackground`, `list.inactiveSelectionForeground`, `list.inactiveSelectionIconForeground`, `list.inactiveFocusBackground`, `list.inactiveFocusOutline`, `list.invalidItemForeground`, `list.errorForeground`, `list.warningForeground`, `listFilterWidget.background`, `listFilterWidget.outline`, `listFilterWidget.noMatchesOutline`, `listFilterWidget.shadow`, `list.filterMatchBackground`, `list.filterMatchBorder`, `list.deemphasizedForeground`, `list.dropBetweenBackground`, `tree.indentGuidesStroke`, `tree.inactiveIndentGuidesStroke`, `tree.tableColumnsBorder`, `tree.tableOddRowsBackground`

**Markdown**（5 · Markdown 预览中的提示块（D5））：`markdownAlert.note.foreground`, `markdownAlert.tip.foreground`, `markdownAlert.important.foreground`, `markdownAlert.warning.foreground`, `markdownAlert.caution.foreground`

**Menu Bar colors**（10）：`menubar.selectionForeground`, `menubar.selectionBackground`, `menubar.selectionBorder`, `menu.foreground`, `menu.background`, `menu.selectionForeground`, `menu.selectionBackground`, `menu.selectionBorder`, `menu.separatorBackground`, `menu.border`

**Notification colors**（11）：`notificationCenter.border`, `notificationCenterHeader.foreground`, `notificationCenterHeader.background`, `notificationToast.border`, `notifications.foreground`, `notifications.background`, `notifications.border`, `notificationLink.foreground`, `notificationsErrorIcon.foreground`, `notificationsWarningIcon.foreground`, `notificationsInfoIcon.foreground`

**Panel colors**（20）：`panel.background`, `panel.border`, `panel.dropBorder`, `panelTitle.activeBorder`, `panelTitle.activeForeground`, `panelTitle.inactiveForeground`, `panelTitle.border`, `panelTitleBadge.background`, `panelTitleBadge.foreground`, `panelInput.border`, `panelSection.border`, `panelSection.dropBackground`, `panelSectionHeader.background`, `panelSectionHeader.foreground`, `panelStickyScroll.background`, `panelStickyScroll.border`, `panelStickyScroll.shadow`, `panelSectionHeader.border`, `outputView.background`, `outputViewStickyScroll.background`

**Peek view colors**（16）：`peekView.border`, `peekViewEditor.background`, `peekViewEditorGutter.background`, `peekViewEditor.matchHighlightBackground`, `peekViewEditor.matchHighlightBorder`, `peekViewResult.background`, `peekViewResult.fileForeground`, `peekViewResult.lineForeground`, `peekViewResult.matchHighlightBackground`, `peekViewResult.selectionBackground`, `peekViewResult.selectionForeground`, `peekViewTitle.background`, `peekViewTitleDescription.foreground`, `peekViewTitleLabel.foreground`, `peekViewEditorStickyScroll.background`, `peekViewEditorStickyScrollGutter.background`

**Progress bar**（1）：`progressBar.background`

**Quick picker colors**（8）：`pickerGroup.border`, `pickerGroup.foreground`, `quickInput.background`, `quickInput.foreground`, `quickInputList.focusBackground`, `quickInputList.focusForeground`, `quickInputList.focusIconForeground`, `quickInputTitle.background`

**Scrollbar control**（5）：`scrollbar.background`, `scrollbar.shadow`, `scrollbarSlider.activeBackground`, `scrollbarSlider.background`, `scrollbarSlider.hoverBackground`

**Side Bar**（14）：`sideBar.background`, `sideBar.foreground`, `sideBar.border`, `sideBar.dropBackground`, `sideBarTitle.foreground`, `sideBarSectionHeader.background`, `sideBarSectionHeader.foreground`, `sideBarSectionHeader.border`, `sideBarActivityBarTop.border`, `sideBarTitle.background`, `sideBarTitle.border`, `sideBarStickyScroll.background`, `sideBarStickyScroll.border`, `sideBarStickyScroll.shadow`

**Status Bar colors**（24）：`statusBar.background`, `statusBar.foreground`, `statusBar.border`, `statusBar.noFolderForeground`, `statusBar.noFolderBackground`, `statusBar.noFolderBorder`, `statusBarItem.activeBackground`, `statusBarItem.hoverForeground`, `statusBarItem.hoverBackground`, `statusBarItem.prominentForeground`, `statusBarItem.prominentBackground`, `statusBarItem.prominentHoverForeground`, `statusBarItem.prominentHoverBackground`, `statusBarItem.errorBackground`, `statusBarItem.errorForeground`, `statusBarItem.errorHoverBackground`, `statusBarItem.errorHoverForeground`, `statusBarItem.warningBackground`, `statusBarItem.warningForeground`, `statusBarItem.warningHoverBackground`, `statusBarItem.warningHoverForeground`, `statusBarItem.compactHoverBackground`, `statusBarItem.focusBorder`, `statusBar.focusBorder`

**Symbol Icons colors**（33）：`symbolIcon.arrayForeground`, `symbolIcon.booleanForeground`, `symbolIcon.classForeground`, `symbolIcon.colorForeground`, `symbolIcon.constantForeground`, `symbolIcon.constructorForeground`, `symbolIcon.enumeratorForeground`, `symbolIcon.enumeratorMemberForeground`, `symbolIcon.eventForeground`, `symbolIcon.fieldForeground`, `symbolIcon.fileForeground`, `symbolIcon.folderForeground`, `symbolIcon.functionForeground`, `symbolIcon.interfaceForeground`, `symbolIcon.keyForeground`, `symbolIcon.keywordForeground`, `symbolIcon.methodForeground`, `symbolIcon.moduleForeground`, `symbolIcon.namespaceForeground`, `symbolIcon.nullForeground`, `symbolIcon.numberForeground`, `symbolIcon.objectForeground`, `symbolIcon.operatorForeground`, `symbolIcon.packageForeground`, `symbolIcon.propertyForeground`, `symbolIcon.referenceForeground`, `symbolIcon.snippetForeground`, `symbolIcon.stringForeground`, `symbolIcon.structForeground`, `symbolIcon.textForeground`, `symbolIcon.typeParameterForeground`, `symbolIcon.unitForeground`, `symbolIcon.variableForeground`

**Text colors**（9）：`textBlockQuote.background`, `textBlockQuote.border`, `textCodeBlock.background`, `textLink.activeForeground`, `textLink.foreground`, `textPreformat.foreground`, `textPreformat.background`, `textPreformat.border`, `textSeparator.foreground`

**Title Bar colors**（5）：`titleBar.activeBackground`, `titleBar.activeForeground`, `titleBar.inactiveBackground`, `titleBar.inactiveForeground`, `titleBar.border`

**Welcome page colors**（3 · 只用到“开始”和“最近”相关的颜色）：`welcomePage.background`, `welcomePage.tileBackground`, `welcomePage.tileHoverBackground`

**Window border**（2）：`window.activeBorder`, `window.inactiveBorder`

## 4. 支持（P1）

**Editor colors**（55）：`editorMultiCursor.primary.foreground`, `editorMultiCursor.primary.background`, `editorMultiCursor.secondary.foreground`, `editorMultiCursor.secondary.background`, `editor.wordHighlightBackground`, `editor.wordHighlightBorder`, `editor.wordHighlightStrongBackground`, `editor.wordHighlightStrongBorder`, `editor.wordHighlightTextBackground`, `editor.wordHighlightTextBorder`, `editorLink.activeForeground`, `editorRuler.foreground`, `editorBracketHighlight.foreground1`, `editorBracketHighlight.foreground2`, `editorBracketHighlight.foreground3`, `editorBracketHighlight.foreground4`, `editorBracketHighlight.foreground5`, `editorBracketHighlight.foreground6`, `editorBracketHighlight.unexpectedBracket.foreground`, `editorBracketPairGuide.activeBackground1`, `editorBracketPairGuide.activeBackground2`, `editorBracketPairGuide.activeBackground3`, `editorBracketPairGuide.activeBackground4`, `editorBracketPairGuide.activeBackground5`, `editorBracketPairGuide.activeBackground6`, `editorBracketPairGuide.background1`, `editorBracketPairGuide.background2`, `editorBracketPairGuide.background3`, `editorBracketPairGuide.background4`, `editorBracketPairGuide.background5`, `editorBracketPairGuide.background6`, `editorOverviewRuler.background`, `editorOverviewRuler.border`, `editorOverviewRuler.findMatchForeground`, `editorOverviewRuler.rangeHighlightForeground`, `editorOverviewRuler.selectionHighlightForeground`, `editorOverviewRuler.wordHighlightForeground`, `editorOverviewRuler.wordHighlightStrongForeground`, `editorOverviewRuler.wordHighlightTextForeground`, `editorOverviewRuler.errorForeground`, `editorOverviewRuler.warningForeground`, `editorOverviewRuler.infoForeground`, `editorOverviewRuler.bracketMatchForeground`, `editorError.foreground`, `editorError.border`, `editorError.background`, `editorWarning.foreground`, `editorWarning.border`, `editorWarning.background`, `editorInfo.foreground`, `editorInfo.border`, `editorInfo.background`, `editorHint.foreground`, `editorHint.border`, `editorUnnecessaryCode.opacity`

**Editor widget colors**（12）：`editorStickyScroll.background`, `editorStickyScroll.border`, `editorStickyScroll.shadow`, `editorStickyScrollGutter.background`, `editorStickyScrollHover.background`, `editorMarkerNavigation.background`, `editorMarkerNavigationError.background`, `editorMarkerNavigationWarning.background`, `editorMarkerNavigationInfo.background`, `editorMarkerNavigationError.headerBackground`, `editorMarkerNavigationWarning.headerBackground`, `editorMarkerNavigationInfo.headerBackground`

**Minimap**（11）：`minimap.findMatchHighlight`, `minimap.selectionHighlight`, `minimap.errorHighlight`, `minimap.warningHighlight`, `minimap.background`, `minimap.selectionOccurrenceHighlight`, `minimap.foregroundOpacity`, `minimap.infoHighlight`, `minimapSlider.background`, `minimapSlider.hoverBackground`, `minimapSlider.activeBackground`

**Settings Editor colors**（21）：`settings.headerForeground`, `settings.modifiedItemIndicator`, `settings.dropdownBackground`, `settings.dropdownForeground`, `settings.dropdownBorder`, `settings.dropdownListBorder`, `settings.checkboxBackground`, `settings.checkboxForeground`, `settings.checkboxBorder`, `settings.rowHoverBackground`, `settings.textInputBackground`, `settings.textInputForeground`, `settings.textInputBorder`, `settings.numberInputBackground`, `settings.numberInputForeground`, `settings.numberInputBorder`, `settings.focusedRowBackground`, `settings.focusedRowBorder`, `settings.headerBorder`, `settings.sashBorder`, `settings.settingsHeaderHoverForeground`

**Simple Find Widget colors**（1 · Markdown 预览（WebView2）中的查找条）：`simpleFindWidget.sashBorder`

## 5. 支持（P2）

**Diff editor colors**（21）：`diffEditor.insertedTextBackground`, `diffEditor.insertedTextBorder`, `diffEditor.removedTextBackground`, `diffEditor.removedTextBorder`, `diffEditor.border`, `diffEditor.diagonalFill`, `diffEditor.insertedLineBackground`, `diffEditor.removedLineBackground`, `diffEditorGutter.insertedLineBackground`, `diffEditorGutter.removedLineBackground`, `diffEditorOverview.insertedForeground`, `diffEditorOverview.removedForeground`, `diffEditor.unchangedRegionBackground`, `diffEditor.unchangedRegionForeground`, `diffEditor.unchangedRegionShadow`, `diffEditor.unchangedCodeBackground`, `diffEditor.move.border`, `diffEditor.moveActive.border`, `multiDiffEditor.headerBackground`, `multiDiffEditor.background`, `multiDiffEditor.border`

**Editor colors**（7）：`editor.placeholder.foreground`, `searchEditor.findMatchBackground`, `searchEditor.findMatchBorder`, `searchEditor.textInputBorder`, `editorUnicodeHighlight.border`, `editorUnicodeHighlight.background`, `editor.linkedEditingBackground`

**Keyboard shortcut table colors**（2）：`keybindingTable.headerBackground`, `keybindingTable.rowsBackground`

## 6. 不支持（按分区）

<details><summary>Action colors：4</summary>

`editorActionList.background`, `editorActionList.foreground`, `editorActionList.focusForeground`, `editorActionList.focusBackground`

</details>

<details><summary>Agent Session colors：5</summary>

`agentSessionReadIndicator.foreground`, `agentSessionSelectedBadge.border`, `agentSessionSelectedUnfocusedBadge.border`, `agentStatusIndicator.background`, `aiCustomizationManagement.sashBorder`

</details>

<details><summary>Agent sessions colors：32</summary>

`agents.background`, `agentsPanel.background`, `agentsPanel.foreground`, `agentsPanel.border`, `agentsCard.border`, `agentsBottomPanel.border`, `agentsGradient.tintColor`, `agentFeedbackEditorWidget.background`, `agentFeedbackEditorWidget.border`, `agentFeedbackInputWidget.border`, `agentsUpdateButton.downloadingBackground`, `agentsUpdateButton.downloadedBackground`, `agentsChatInput.background`, `agentsChatInput.foreground`, `agentsChatInput.border`, `agentsChatInput.focusBorder`, `agentsChatInput.placeholderForeground`, `agentsNewSessionButton.background`, `agentsNewSessionButton.foreground`, `agentsNewSessionButton.border`, `agentsNewSessionButton.hoverBackground`, `agentsBadge.background`, `agentsBadge.foreground`, `agentsUnreadBadge.background`, `agentsUnreadBadge.foreground`, `activeSessionView.background`, `inactiveSessionView.background`, `activeSessionView.foreground`, `inactiveSessionView.foreground`, `agentsMobileDiff.addedForeground`, `agentsMobileDiff.modifiedForeground`, `agentsMobileDiff.deletedForeground`

</details>

<details><summary>Banner colors：3</summary>

`banner.background`, `banner.foreground`, `banner.iconForeground`

</details>

<details><summary>Chart colors：11</summary>

`charts.foreground`, `charts.lines`, `charts.red`, `charts.blue`, `charts.yellow`, `charts.orange`, `charts.green`, `charts.purple`, `chart.line`, `chart.axis`, `chart.guide`

</details>

<details><summary>Chat colors：15</summary>

`chat.requestBorder`, `chat.requestBackground`, `chat.slashCommandBackground`, `chat.slashCommandForeground`, `chat.avatarBackground`, `chat.avatarForeground`, `chat.editedFileForeground`, `chat.linesAddedForeground`, `chat.linesRemovedForeground`, `chat.requestCodeBorder`, `chat.requestBubbleBackground`, `chat.requestBubbleHoverBackground`, `chat.checkpointSeparator`, `chat.thinkingShimmer`, `chatManagement.sashBorder`

</details>

<details><summary>Command Center colors：1</summary>

`commandCenter.debuggingBackground`

</details>

<details><summary>Comments View colors：2</summary>

`commentsView.resolvedIcon`, `commentsView.unresolvedIcon`

</details>

<details><summary>Debug Icons colors：20</summary>

`debugIcon.breakpointForeground`, `debugIcon.breakpointDisabledForeground`, `debugIcon.breakpointUnverifiedForeground`, `debugIcon.breakpointCurrentStackframeForeground`, `debugIcon.breakpointStackframeForeground`, `debugIcon.startForeground`, `debugIcon.pauseForeground`, `debugIcon.stopForeground`, `debugIcon.disconnectForeground`, `debugIcon.restartForeground`, `debugIcon.stepOverForeground`, `debugIcon.stepIntoForeground`, `debugIcon.stepOutForeground`, `debugIcon.continueForeground`, `debugIcon.stepBackForeground`, `debugConsole.infoForeground`, `debugConsole.warningForeground`, `debugConsole.errorForeground`, `debugConsole.sourceForeground`, `debugConsoleInputIcon.foreground`

</details>

<details><summary>Debug colors：18</summary>

`debugToolBar.background`, `debugToolBar.border`, `editor.stackFrameHighlightBackground`, `editor.focusedStackFrameHighlightBackground`, `editor.inlineValuesForeground`, `editor.inlineValuesBackground`, `debugView.exceptionLabelForeground`, `debugView.exceptionLabelBackground`, `debugView.stateLabelForeground`, `debugView.stateLabelBackground`, `debugView.valueChangedHighlight`, `debugTokenExpression.name`, `debugTokenExpression.value`, `debugTokenExpression.string`, `debugTokenExpression.boolean`, `debugTokenExpression.number`, `debugTokenExpression.error`, `debugTokenExpression.type`

</details>

<details><summary>Editor colors：52</summary>

`editorInlayHint.background`, `editorInlayHint.foreground`, `editorInlayHint.typeForeground`, `editorInlayHint.typeBackground`, `editorInlayHint.parameterForeground`, `editorInlayHint.parameterBackground`, `editorCodeLens.foreground`, `editorLightBulb.foreground`, `editorLightBulbAutoFix.foreground`, `editorLightBulbAi.foreground`, `editorOverviewRuler.modifiedForeground`, `editorOverviewRuler.addedForeground`, `editorOverviewRuler.deletedForeground`, `editorOverviewRuler.inlineChatInserted`, `editorOverviewRuler.inlineChatRemoved`, `editorOverviewRuler.commentDraftForeground`, `editorGutter.modifiedBackground`, `editorGutter.modifiedSecondaryBackground`, `editorGutter.addedBackground`, `editorGutter.addedSecondaryBackground`, `editorGutter.deletedBackground`, `editorGutter.deletedSecondaryBackground`, `editorGutter.commentRangeForeground`, `editorGutter.commentGlyphForeground`, `editorGutter.commentUnresolvedGlyphForeground`, `editorGutter.itemGlyphForeground`, `editorGutter.itemBackground`, `editorCommentsWidget.resolvedBorder`, `editorCommentsWidget.unresolvedBorder`, `editorCommentsWidget.rangeBackground`, `editorCommentsWidget.rangeActiveBackground`, `editorCommentsWidget.replyInputBackground`, `inlineEdit.gutterIndicator.primaryBorder`, `inlineEdit.gutterIndicator.primaryForeground`, `inlineEdit.gutterIndicator.primaryBackground`, `inlineEdit.gutterIndicator.secondaryBorder`, `inlineEdit.gutterIndicator.secondaryForeground`, `inlineEdit.gutterIndicator.secondaryBackground`, `inlineEdit.gutterIndicator.successfulBorder`, `inlineEdit.gutterIndicator.successfulForeground`, `inlineEdit.gutterIndicator.successfulBackground`, `inlineEdit.gutterIndicator.background`, `inlineEdit.originalBackground`, `inlineEdit.modifiedBackground`, `inlineEdit.originalChangedLineBackground`, `inlineEdit.originalChangedTextBackground`, `inlineEdit.modifiedChangedLineBackground`, `inlineEdit.modifiedChangedTextBackground`, `inlineEdit.originalBorder`, `inlineEdit.modifiedBorder`, `inlineEdit.tabWillAcceptModifiedBorder`, `inlineEdit.tabWillAcceptOriginalBorder`

</details>

<details><summary>Editor widget colors：15</summary>

`editorSuggestWidget.background`, `editorSuggestWidget.border`, `editorSuggestWidget.foreground`, `editorSuggestWidget.focusHighlightForeground`, `editorSuggestWidget.highlightForeground`, `editorSuggestWidget.selectedBackground`, `editorSuggestWidget.selectedForeground`, `editorSuggestWidget.selectedIconForeground`, `editorSuggestWidgetStatus.foreground`, `editorHoverWidget.highlightForeground`, `editorGhostText.border`, `editorGhostText.background`, `editorGhostText.foreground`, `debugExceptionWidget.background`, `debugExceptionWidget.border`

</details>

<details><summary>Extensions colors：16</summary>

`extensionButton.prominentForeground`, `extensionButton.prominentBackground`, `extensionButton.prominentHoverBackground`, `extensionButton.background`, `extensionButton.foreground`, `extensionButton.hoverBackground`, `extensionButton.separator`, `extensionButton.border`, `extensionBadge.remoteBackground`, `extensionBadge.remoteForeground`, `extensionIcon.starForeground`, `extensionIcon.verifiedForeground`, `extensionIcon.preReleaseForeground`, `extensionIcon.sponsorForeground`, `extensionIcon.privateForeground`, `mcpIcon.starForeground`

</details>

<details><summary>Gauge colors：7</summary>

`gauge.background`, `gauge.foreground`, `gauge.border`, `gauge.warningBackground`, `gauge.warningForeground`, `gauge.errorBackground`, `gauge.errorForeground`

</details>

<details><summary>Git colors：11</summary>

`gitDecoration.addedResourceForeground`, `gitDecoration.modifiedResourceForeground`, `gitDecoration.deletedResourceForeground`, `gitDecoration.renamedResourceForeground`, `gitDecoration.stageModifiedResourceForeground`, `gitDecoration.stageDeletedResourceForeground`, `gitDecoration.untrackedResourceForeground`, `gitDecoration.ignoredResourceForeground`, `gitDecoration.conflictingResourceForeground`, `gitDecoration.submoduleResourceForeground`, `git.blame.editorDecorationForeground`

</details>

<details><summary>Inline Chat colors：10</summary>

`inlineChat.background`, `inlineChat.foreground`, `inlineChat.border`, `inlineChat.shadow`, `inlineChatInput.border`, `inlineChatInput.focusBorder`, `inlineChatInput.placeholderForeground`, `inlineChatInput.background`, `inlineChatDiff.inserted`, `inlineChatDiff.removed`

</details>

<details><summary>Integrated Terminal colors：61</summary>

`terminal.background`, `terminal.border`, `terminal.foreground`, `terminal.ansiBlack`, `terminal.ansiBlue`, `terminal.ansiBrightBlack`, `terminal.ansiBrightBlue`, `terminal.ansiBrightCyan`, `terminal.ansiBrightGreen`, `terminal.ansiBrightMagenta`, `terminal.ansiBrightRed`, `terminal.ansiBrightWhite`, `terminal.ansiBrightYellow`, `terminal.ansiCyan`, `terminal.ansiGreen`, `terminal.ansiMagenta`, `terminal.ansiRed`, `terminal.ansiWhite`, `terminal.ansiYellow`, `terminal.selectionBackground`, `terminal.selectionForeground`, `terminal.inactiveSelectionBackground`, `terminal.findMatchBackground`, `terminal.findMatchBorder`, `terminal.findMatchHighlightBackground`, `terminal.findMatchHighlightBorder`, `terminal.hoverHighlightBackground`, `terminalCursor.background`, `terminalCursor.foreground`, `terminal.dropBackground`, `terminal.tab.activeBorder`, `terminalCommandDecoration.defaultBackground`, `terminalCommandDecoration.successBackground`, `terminalCommandDecoration.errorBackground`, `terminalOverviewRuler.cursorForeground`, `terminalOverviewRuler.findMatchForeground`, `terminalStickyScroll.background`, `terminalStickyScroll.border`, `terminalStickyScrollHover.background`, `terminal.initialHintForeground`, `terminalOverviewRuler.border`, `terminalCommandGuide.foreground`, `terminalSymbolIcon.aliasForeground`, `terminalSymbolIcon.branchForeground`, `terminalSymbolIcon.commitForeground`, `terminalSymbolIcon.flagForeground`, `terminalSymbolIcon.optionForeground`, `terminalSymbolIcon.optionValueForeground`, `terminalSymbolIcon.methodForeground`, `terminalSymbolIcon.argumentForeground`, `terminalSymbolIcon.inlineSuggestionForeground`, `terminalSymbolIcon.fileForeground`, `terminalSymbolIcon.folderForeground`, `terminalSymbolIcon.pullRequestDoneForeground`, `terminalSymbolIcon.pullRequestForeground`, `terminalSymbolIcon.remoteForeground`, `terminalSymbolIcon.stashForeground`, `terminalSymbolIcon.symbolText`, `terminalSymbolIcon.symbolicLinkFileForeground`, `terminalSymbolIcon.symbolicLinkFolderForeground`, `terminalSymbolIcon.tagForeground`

</details>

<details><summary>Merge conflicts colors：25</summary>

`merge.currentHeaderBackground`, `merge.currentContentBackground`, `merge.incomingHeaderBackground`, `merge.incomingContentBackground`, `merge.border`, `merge.commonContentBackground`, `merge.commonHeaderBackground`, `editorOverviewRuler.currentContentForeground`, `editorOverviewRuler.incomingContentForeground`, `editorOverviewRuler.commonContentForeground`, `editorOverviewRuler.commentForeground`, `editorOverviewRuler.commentUnresolvedForeground`, `mergeEditor.change.background`, `mergeEditor.change.word.background`, `mergeEditor.conflict.unhandledUnfocused.border`, `mergeEditor.conflict.unhandledFocused.border`, `mergeEditor.conflict.handledUnfocused.border`, `mergeEditor.conflict.handledFocused.border`, `mergeEditor.conflict.handled.minimapOverViewRuler`, `mergeEditor.conflict.unhandled.minimapOverViewRuler`, `mergeEditor.conflictingLines.background`, `mergeEditor.changeBase.background`, `mergeEditor.changeBase.word.background`, `mergeEditor.conflict.input1.background`, `mergeEditor.conflict.input2.background`

</details>

<details><summary>Minimap：5</summary>

`minimap.chatEditHighlight`, `minimapGutter.addedBackground`, `minimapGutter.modifiedBackground`, `minimapGutter.deletedBackground`, `editorMinimap.inlineChatInserted`

</details>

<details><summary>Modern UI colors：29</summary>

`surface.background`, `surface.foreground`, `surface.border`, `editor.border`, `modernPanel.border`, `modernSash.gripForeground`, `modernTab.activeBackground`, `modernTab.activeForeground`, `modernTab.hoverBackground`, `modernTab.hoverForeground`, `modernEditorTab.activeBackground`, `modernEditorTab.activeActionBackground`, `modernEditorTab.activeForeground`, `modernEditorTab.inactiveBackground`, `modernEditorTab.hoverBackground`, `modernEditorTab.hoverActionBackground`, `modernEditorTab.hoverForeground`, `modernEditorTab.activeHoverBackground`, `modernEditorTab.activeHoverActionBackground`, `modernEditorTab.selectedActionBackground`, `modernActivityBar.background`, `modernActivityBar.inactiveBackground`, `modernActivityBarItem.activeBackground`, `modernActivityBarItem.activeForeground`, `modernActivityBarItem.hoverBackground`, `modernActivityBarItem.hoverForeground`, `modernActivityBar.border`, `modernUI.shellBackground`, `modernUI.inactiveShellBackground`

</details>

<details><summary>Notebook colors：24</summary>

`notebook.editorBackground`, `notebook.cellBorderColor`, `notebook.cellHoverBackground`, `notebook.cellInsertionIndicator`, `notebook.cellStatusBarItemHoverBackground`, `notebook.cellToolbarSeparator`, `notebook.cellEditorBackground`, `notebook.focusedCellBackground`, `notebook.focusedCellBorder`, `notebook.focusedEditorBorder`, `notebook.inactiveFocusedCellBorder`, `notebook.inactiveSelectedCellBorder`, `notebook.outputContainerBackgroundColor`, `notebook.outputContainerBorderColor`, `notebook.selectedCellBackground`, `notebook.selectedCellBorder`, `notebook.symbolHighlightBackground`, `notebookScrollbarSlider.activeBackground`, `notebookScrollbarSlider.background`, `notebookScrollbarSlider.hoverBackground`, `notebookStatusErrorIcon.foreground`, `notebookStatusRunningIcon.foreground`, `notebookStatusSuccessIcon.foreground`, `notebookEditorOverviewRuler.runningCellForeground`

</details>

<details><summary>Panel Chat colors：2</summary>

`interactive.activeCodeBorder`, `interactive.inactiveCodeBorder`

</details>

<details><summary>Ports colors：1</summary>

`ports.iconRunningProcessForeground`

</details>

<details><summary>Profiles：3</summary>

`profileBadge.background`, `profileBadge.foreground`, `profiles.sashBorder`

</details>

<details><summary>Snippets colors：4</summary>

`editor.snippetTabstopHighlightBackground`, `editor.snippetTabstopHighlightBorder`, `editor.snippetFinalTabstopHighlightBackground`, `editor.snippetFinalTabstopHighlightBorder`

</details>

<details><summary>Source Control Graph colors：13</summary>

`scmGraph.historyItemHoverLabelForeground`, `scmGraph.foreground1`, `scmGraph.foreground2`, `scmGraph.foreground3`, `scmGraph.foreground4`, `scmGraph.foreground5`, `scmGraph.historyItemHoverAdditionsForeground`, `scmGraph.historyItemHoverDeletionsForeground`, `scmGraph.historyItemRefColor`, `scmGraph.historyItemRemoteRefColor`, `scmGraph.historyItemBaseRefColor`, `scmGraph.historyItemHoverDefaultLabelForeground`, `scmGraph.historyItemHoverDefaultLabelBackground`

</details>

<details><summary>Status Bar colors：11</summary>

`statusBar.debuggingBackground`, `statusBar.debuggingForeground`, `statusBar.debuggingBorder`, `statusBarItem.remoteBackground`, `statusBarItem.remoteForeground`, `statusBarItem.remoteHoverBackground`, `statusBarItem.remoteHoverForeground`, `statusBarItem.offlineBackground`, `statusBarItem.offlineForeground`, `statusBarItem.offlineHoverForeground`, `statusBarItem.offlineHoverBackground`

</details>

<details><summary>Testing colors：32</summary>

`testing.runAction`, `testing.iconErrored`, `testing.iconFailed`, `testing.iconPassed`, `testing.iconQueued`, `testing.iconUnset`, `testing.iconSkipped`, `testing.iconErrored.retired`, `testing.iconFailed.retired`, `testing.iconPassed.retired`, `testing.iconQueued.retired`, `testing.iconUnset.retired`, `testing.iconSkipped.retired`, `testing.peekBorder`, `testing.peekHeaderBackground`, `testing.message.error.lineBackground`, `testing.message.info.decorationForeground`, `testing.message.info.lineBackground`, `testing.messagePeekBorder`, `testing.messagePeekHeaderBackground`, `testing.coveredBackground`, `testing.coveredBorder`, `testing.coveredGutterBackground`, `testing.uncoveredBranchBackground`, `testing.uncoveredBackground`, `testing.uncoveredBorder`, `testing.uncoveredGutterBackground`, `testing.coverCountBadgeBackground`, `testing.coverCountBadgeForeground`, `testing.message.error.badgeBackground`, `testing.message.error.badgeBorder`, `testing.message.error.badgeForeground`

</details>

<details><summary>Welcome page colors：5</summary>

`welcomePage.progress.background`, `welcomePage.progress.foreground`, `welcomePage.tileBorder`, `walkThrough.embeddedEditorBackground`, `walkthrough.stepTitle.foreground`

</details>
