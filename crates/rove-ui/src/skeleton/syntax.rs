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
/// tree-sitter 的行号只在 LF 上递增，而 #7 的缓冲区把单独的 CR 也当作换行；
/// 直接拿 `node.start_position().row` 去索引 `line_range` 会让 CR-only / 混合换行
/// 文件的后半段 token 全部错位或丢失。统一按字节偏移反查缓冲区的行号。
///
/// 节点边界可能落在行终止符或 UTF-8 标量内部，此时向前找最近的合法位置。
fn line_of_byte(text: &dyn TextSource, offset: u64) -> Result<Line> {
    let mut offset = offset.min(text.byte_len());
    loop {
        match text.position(ByteOffset(offset)) {
            Ok(position) => return Ok(position.line),
            Err(Error::OutOfBounds) if offset > 0 => offset -= 1,
            Err(error) => return Err(error),
        }
    }
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
                    let start = line_of_byte(text.as_ref(), c.node.start_byte() as u64)?;
                    let end = line_of_byte(text.as_ref(), c.node.end_byte() as u64)?;
                    let scope: Arc<str> = query.capture_names()[c.index as usize].into();
                    for row in start.0..=end.0 {
                        let row_index = row as usize;
                        if row_index >= lines.len() {
                            break;
                        }
                        let line = text.line_range(Line(row))?;
                        let a = line.start().0.max(c.node.start_byte() as u64);
                        let b = line.end().0.min(c.node.end_byte() as u64);
                        if a < b {
                            lines[row_index].push(Token {
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

#[cfg(test)]
mod tests {
    use super::*;
    use rove_buffer::RopeText;
    use std::task::{Context, Poll, Waker};

    /// `analyze` 不 await 任何东西，一次 poll 就能完成；测试里不需要完整 executor。
    fn block_on<T>(mut task: Task<T>) -> Result<T> {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        loop {
            if let Poll::Ready(value) = task.as_mut().poll(&mut context) {
                return value;
            }
        }
    }

    fn has_keyword(snapshot: &Arc<dyn SyntaxSnapshot>, line: Line) -> bool {
        snapshot
            .tokens(line)
            .unwrap()
            .iter()
            .any(|token| token.scopes.iter().any(|scope| scope.as_ref() == "keyword"))
    }

    #[test]
    fn standalone_cr_lines_keep_highlights() {
        // tree-sitter 只在 LF 上递增 row；缓冲区把单独的 CR 当作换行，
        // 所以第二行必须按字节偏移反查，而不是用 `start_position().row`。
        let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text("class A {}\rclass B {}\r"));
        let update =
            block_on(CSharp.analyze(text, "csharp".into(), Cancellation::default())).unwrap();
        assert!(has_keyword(&update.snapshot, Line(0)));
        assert!(
            has_keyword(&update.snapshot, Line(1)),
            "CR-only 文件的第二行必须保留 keyword token"
        );
    }
}
