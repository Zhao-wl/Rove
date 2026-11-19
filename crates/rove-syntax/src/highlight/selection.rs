//! 从当前解析结果读取严格嵌套的选择区间。

use super::{DocumentSyntax, ParseTrees};
use crate::SyntaxSnapshot;
use rove_base::{ByteOffset, ByteRange, Span, Version};

impl DocumentSyntax {
    pub(super) fn selection_ranges(
        &self,
        version: Version,
        language: &str,
        selection: &ByteRange,
    ) -> Vec<ByteRange> {
        let snapshot = self
            .baseline
            .try_lock()
            .ok()
            .and_then(|baseline| baseline.clone());
        let Some(snapshot) = snapshot
            .filter(|snapshot| snapshot.version() == version && &*snapshot.language == language)
        else {
            return Vec::new();
        };
        let Some(parsed) = &snapshot.parsed else {
            return Vec::new();
        };
        let Some((start, end)) = usize::try_from(selection.start().0)
            .ok()
            .zip(usize::try_from(selection.end().0).ok())
        else {
            return Vec::new();
        };
        let tree = match &parsed.trees {
            ParseTrees::Whole(tree) => Some(tree),
            ParseTrees::Csharp(parts) => parts.selection_tree(start),
        };
        let Some(root) = tree
            .map(|tree| tree.root_node())
            .filter(|root| end <= root.end_byte())
        else {
            return Vec::new();
        };
        let Some(mut node) = root.descendant_for_byte_range(start, end) else {
            return Vec::new();
        };
        let mut previous = start..end;
        let mut ranges = Vec::new();
        loop {
            let bytes = node.byte_range();
            if node.is_named()
                && !node.is_missing()
                && bytes != previous
                && bytes.start <= previous.start
                && previous.end <= bytes.end
            {
                if let Ok(range) =
                    Span::new(ByteOffset(bytes.start as u64), ByteOffset(bytes.end as u64))
                {
                    ranges.push(range);
                    previous = bytes;
                }
            }
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use crate::highlight::{DocumentHighlighter, SyntaxDocuments, TreeSitterSyntax};
    use rove_base::{ByteOffset, Cancellation, DocumentId, Span, Version};
    use rove_buffer::{RopeText, TextSource};
    use std::sync::Arc;

    #[test]
    fn uses_real_named_ancestors_for_supported_grammars() {
        let documents = SyntaxDocuments::new(TreeSitterSyntax::default());
        for (index, (language, source, marker, expected)) in [
            (
                "csharp",
                "class C { int M() { return value + 1; } }",
                "value",
                "return value + 1;",
            ),
            (
                "json",
                r#"{"root": {"name":"value"}}"#,
                "value",
                r#""name":"value""#,
            ),
            (
                "yaml",
                "root:\n  name: value\n  kind: test\n",
                "value",
                "name: value",
            ),
            (
                "xml",
                "<root><child>value</child></root>",
                "value",
                "<child>value</child>",
            ),
            (
                "markdown",
                "# Title\n\nRead **value** here.\n",
                "value",
                "Read **value** here.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let id = DocumentId(index as u64);
            let text = Arc::new(RopeText::from_text(source));
            let version = text.version();
            futures_lite::future::block_on(documents.analyze(
                id,
                text,
                language.into(),
                None,
                None,
                Cancellation::default(),
            ))
            .unwrap();
            let column = ByteOffset((source.find(marker).unwrap() + 1) as u64);
            let caret = Span::new(column, column).unwrap();
            let ranges = documents.selection_ranges(id, version, language, &caret);
            let strings: Vec<_> = ranges
                .iter()
                .map(|range| &source[range.start().0 as usize..range.end().0 as usize])
                .collect();
            assert!(strings.contains(&expected), "{language} {strings:?}");
            assert!(ranges
                .windows(2)
                .all(|pair| pair[1].start() <= pair[0].start()
                    && pair[0].end() <= pair[1].end()
                    && pair[0] != pair[1]));
            assert!(documents
                .selection_ranges(id, Version(version.0 + 1), language, &caret)
                .is_empty());
            assert!(documents
                .selection_ranges(id, version, "plaintext", &caret)
                .is_empty());
            documents.close(id);
            assert!(documents
                .selection_ranges(id, version, language, &caret)
                .is_empty());
        }
    }
}
