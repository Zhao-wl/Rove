//! SKELETON(syntax): small tree-sitter C# query; replaced by #9.
use rove_base::*;
use rove_buffer::TextSource;
use rove_syntax::*;
use std::sync::Arc;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};
const QUERY: &str = r#"
["class" "struct" "interface" "namespace" "using" "public" "private" "static" "return" "new"] @keyword
(predefined_type) @keyword
(comment) @comment
(string_literal) @string
(integer_literal) @constant.numeric
"#;
pub struct CSharp;
struct Highlight {
    version: Version,
    lines: Vec<Arc<[Token]>>,
}
impl SyntaxSnapshot for Highlight {
    fn version(&self) -> Version {
        self.version
    }
    fn tokens(&self, l: Line) -> Result<Arc<[Token]>> {
        self.lines
            .get(l.0 as usize)
            .cloned()
            .ok_or(Error::OutOfBounds)
    }
    fn folds(&self) -> Arc<[Fold]> {
        Arc::from([])
    }
    fn symbols(&self) -> Arc<[Symbol]> {
        Arc::from([])
    }
}
/// Bounded UTF-8 chunk adapter; tree-sitter may request an arbitrary byte within a scalar.
fn chunk(text: &dyn TextSource, offset: usize) -> Result<Vec<u8>> {
    let len = text.byte_len() as usize;
    if offset >= len {
        return Ok(vec![]);
    }
    let mut start = offset;
    let mut end = (offset + 4096).min(len);
    for _ in 0..4 {
        for _ in 0..4 {
            let mut result = vec![];
            if text
                .visit_chunks(
                    Span::new(ByteOffset(start as u64), ByteOffset(end as u64))?,
                    &mut |s| result.extend_from_slice(s.as_bytes()),
                )
                .is_ok()
            {
                return Ok(result[offset - start..].to_vec());
            }
            if end == len {
                break;
            }
            end -= 1;
        }
        if start == 0 {
            break;
        }
        start -= 1;
        end = (offset + 4096).min(len);
    }
    Err(Error::OutOfBounds)
}
impl SyntaxService for CSharp {
    fn analyze(
        &self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        Box::pin(async move {
            cancel.check()?;
            let mut lines: Vec<Vec<Token>> = vec![vec![]; text.lines().known as usize];
            if language.as_ref() == "csharp" {
                let lang = tree_sitter_c_sharp::LANGUAGE.into();
                let mut parser = Parser::new();
                parser
                    .set_language(&lang)
                    .map_err(|_| Error::InvalidInput)?;
                let mut read_error = None;
                let tree = parser
                    .parse_with_options(
                        &mut |offset, _| {
                            if let Err(e) = cancel.check() {
                                read_error = Some(e);
                                return vec![];
                            }
                            match chunk(text.as_ref(), offset) {
                                Ok(s) => s,
                                Err(e) => {
                                    read_error = Some(e);
                                    vec![]
                                }
                            }
                        },
                        None,
                        None,
                    )
                    .ok_or(Error::NotReady)?;
                if let Some(e) = read_error {
                    return Err(e);
                }
                cancel.check()?;
                let query = Query::new(&lang, QUERY).map_err(|e| {
                    eprintln!("C# skeleton query: {e}");
                    Error::InvalidInput
                })?;
                let mut cursor = QueryCursor::new();
                let mut captures = cursor.captures(&query, tree.root_node(), &[] as &[u8]);
                while let Some((m, i)) = captures.next() {
                    cancel.check()?;
                    let c = m.captures[*i];
                    let start = c.node.start_position().row;
                    let end = c.node.end_position().row;
                    let scope: Arc<str> = query.capture_names()[c.index as usize].into();
                    for row in start..=end {
                        if row >= lines.len() {
                            break;
                        }
                        let line = text.line_range(Line(row as u64))?;
                        let a = line.start().0.max(c.node.start_byte() as u64);
                        let b = line.end().0.min(c.node.end_byte() as u64);
                        if a < b {
                            lines[row].push(Token {
                                range: Span::new(ByteOffset(a), ByteOffset(b))?,
                                scopes: Arc::from([scope.clone()]),
                            });
                        }
                    }
                }
            } else if language.as_ref() != "plaintext" {
                return Err(Error::Unsupported);
            }
            for line in &mut lines {
                line.sort_by_key(|t| t.range.start());
            }
            Ok(SyntaxUpdate {
                snapshot: Arc::new(Highlight {
                    version: text.version(),
                    lines: lines.into_iter().map(Arc::from).collect(),
                }),
                previous: None,
                changed_lines: None,
            })
        })
    }
}
