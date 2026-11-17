use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
};

use rove_base::{Line, Position};
use rove_buffer::TextSource;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Link {
    /// UTF-8 byte columns, matching the text source and inline decorations.
    pub range: Range<u32>,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkTarget {
    External {
        url: String,
        display: String,
    },
    Path {
        display: String,
        path: PathBuf,
        position: Option<Position>,
        fragment: Option<String>,
        document_relative: bool,
    },
    Reference(String),
}

impl LinkTarget {
    pub fn display(&self) -> &str {
        match self {
            Self::External { display, .. } | Self::Path { display, .. } => display,
            Self::Reference(label) => label,
        }
    }
}

/// Reads only the visible prefix of one line. A UTF-8 character crossing the byte
/// budget is omitted along with the rest of the line.
pub(super) fn line_prefix(text: &dyn TextSource, line: u64, max_chars: usize) -> String {
    let Ok(span) = text.line_range(Line(line)) else {
        return String::new();
    };
    let max_bytes = max_chars.saturating_mul(4) as u64;
    let end = span.start().0.saturating_add(max_bytes).min(span.end().0);
    let mut end = rove_base::ByteOffset(end);
    let mut result = String::new();
    for _ in 0..4 {
        let Ok(prefix) = rove_base::Span::new(span.start(), end) else {
            break;
        };
        result.clear();
        if text
            .visit_chunks(prefix, &mut |chunk| result.push_str(chunk))
            .is_ok()
        {
            break;
        }
        if end == span.start() {
            break;
        }
        end.0 -= 1;
    }
    if let Some((cut, next)) = result.char_indices().nth(max_chars) {
        result.truncate(cut);
        if !next.is_whitespace() && !matches!(next, '"' | '\'' | '<' | '>') {
            remove_partial_token(&mut result);
        }
    } else if end < span.end() {
        remove_partial_token(&mut result);
    }
    result
}

fn remove_partial_token(prefix: &mut String) {
    let boundary = prefix.char_indices().rev().find_map(|(i, ch)| {
        (ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>')).then_some(i + ch.len_utf8())
    });
    prefix.truncate(boundary.unwrap_or(0));
}

pub(crate) fn scan(line: &str, markdown: bool) -> Vec<Link> {
    let mut links = if markdown {
        markdown_links(line)
    } else {
        Vec::new()
    };
    if markdown {
        let mut additional = Vec::new();
        for (start, end) in angle_spans(line) {
            let next = links.partition_point(|link| link.range.end <= start);
            if links.get(next).is_some_and(|link| link.range.start < end) {
                continue;
            }
            let value = &line[start as usize + 1..end as usize - 1];
            if value.chars().any(char::is_whitespace)
                || (!value.starts_with('#') && !value.contains(['.', '/', '\\']))
            {
                continue;
            }
            if let Some(target) = target(value, true) {
                additional.push(Link {
                    range: start..end,
                    target,
                });
            }
        }
        links.extend(additional);
        links.sort_unstable_by_key(|link| link.range.start);
    }
    let mut quoted = Vec::new();
    let mut quote = None;
    for (offset, byte) in line.bytes().enumerate() {
        if !matches!(byte, b'\'' | b'"') {
            continue;
        }
        match quote {
            Some((start, delimiter)) if byte == delimiter => {
                let range = start..offset;
                let value = &line[range.clone()];
                if value.chars().any(char::is_whitespace) {
                    let start = start as u32;
                    let end = offset as u32;
                    let next = links.partition_point(|link| link.range.end <= start);
                    if links.get(next).is_none_or(|link| link.range.start >= end) {
                        if let Some(target) = target(value, false) {
                            quoted.push(Link {
                                range: start..end,
                                target,
                            });
                        }
                    }
                }
                quote = None;
            }
            None => quote = Some((offset + 1, byte)),
            _ => {}
        }
    }
    links.extend(quoted);
    links.sort_unstable_by_key(|link| link.range.start);
    let structured_count = links.len();
    for (start, word) in words(line) {
        let word = trim_punctuation(word);
        let Some(target) = target(word, false) else {
            continue;
        };
        let range = start as u32..(start + word.len()) as u32;
        let structured = &links[..structured_count];
        let next = structured.partition_point(|link| link.range.end <= range.start);
        if structured
            .get(next)
            .is_none_or(|link| range.end <= link.range.start)
        {
            links.push(Link { range, target });
        }
    }
    if structured_count != 0 {
        links.sort_unstable_by_key(|link| link.range.start);
    }
    links
}

fn words(line: &str) -> impl Iterator<Item = (usize, &str)> {
    line.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '<' | '>' | '\x60' | '“' | '”' | '‘' | '’' | '《' | '》'
            )
    })
    .filter_map(|word| {
        let word = word.trim_start_matches(['(', '[', '{', '（', '【', '：']);
        (!word.is_empty()).then(|| (word.as_ptr() as usize - line.as_ptr() as usize, word))
    })
}

fn trim_punctuation(word: &str) -> &str {
    let word = word.trim_end_matches([
        '.', ',', ';', ':', '!', '?', '。', '，', '；', '：', '！', '？',
    ]);
    let mut end = word.len();
    for (open, close) in [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('（', '）'),
        ('【', '】'),
    ] {
        if word[..end].ends_with(close) {
            let count = |c| word[..end].chars().filter(|ch| *ch == c).count();
            for _ in 0..count(close).saturating_sub(count(open)) {
                if !word[..end].ends_with(close) {
                    break;
                }
                end -= close.len_utf8();
            }
        }
    }
    word[..end].trim_end_matches([
        '.', ',', ';', ':', '!', '?', '。', '，', '；', '：', '！', '？',
    ])
}

fn angle_spans(line: &str) -> impl Iterator<Item = (u32, u32)> + '_ {
    let mut open = None;
    line.bytes()
        .enumerate()
        .filter_map(move |(i, byte)| match byte {
            b'<' => {
                open = Some(i as u32);
                None
            }
            b'>' => open.take().map(|start| (start, i as u32 + 1)),
            _ => None,
        })
}

fn markdown_links(line: &str) -> Vec<Link> {
    use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag};
    fn preserve_unresolved(link: BrokenLink<'_>) -> Option<(CowStr<'_>, CowStr<'_>)> {
        matches!(link.link_type, LinkType::Reference | LinkType::Collapsed)
            .then_some((CowStr::from(""), link.reference))
    }
    Parser::new_with_broken_link_callback(line, Options::empty(), Some(preserve_unresolved))
        .into_offset_iter()
        .filter_map(|(event, span)| {
            let Event::Start(Tag::Link {
                dest_url, title, ..
            }) = event
            else {
                return None;
            };
            let result = if dest_url.is_empty() {
                (!title.is_empty()).then(|| LinkTarget::Reference(title.into_string()))?
            } else {
                target(&dest_url, true)?
            };
            Some(Link {
                range: span.start as u32..span.end as u32,
                target: result,
            })
        })
        .collect()
}

pub(crate) fn target(value: &str, markdown: bool) -> Option<LinkTarget> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    let http = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    let external = http.or_else(|| lower.strip_prefix("www.").map(|_| lower.as_str()));
    if let Some(rest) = external {
        let authority = rest.split(['/', '?', '#']).next()?;
        return (!authority.is_empty()
            && !authority.contains(['\\', '@'])
            && !value.chars().any(char::is_whitespace))
        .then(|| LinkTarget::External {
            url: if http.is_some() {
                value.into()
            } else {
                format!("https://{value}")
            },
            display: value.into(),
        });
    }

    let (path, fragment) = if markdown {
        value
            .split_once('#')
            .map_or((value, None), |(path, fragment)| {
                (
                    path,
                    Some(crate::markdown::render::percent_decode(fragment)),
                )
            })
    } else {
        (value, None)
    };
    let (path, position) = split_position(path);
    let path = if let Some(uri) = path.get(7..).filter(|_| lower.starts_with("file://")) {
        let local = if uri.starts_with('/') {
            uri
        } else {
            let (host, local) = uri.split_once('/')?;
            if !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            // Keep the slash for a POSIX file URL.
            &uri[host.len()..host.len() + 1 + local.len()]
        };
        let local = crate::markdown::render::percent_decode(local);
        let local = local
            .strip_prefix('/')
            .filter(|v| drive_absolute(v))
            .unwrap_or(&local);
        if !local.starts_with('/') && !drive_absolute(local) {
            return None;
        }
        local.to_owned()
    } else if markdown {
        crate::markdown::render::percent_decode(path)
    } else {
        path.to_owned()
    };
    if path.chars().any(char::is_control)
        || path
            .as_bytes()
            .get(..2)
            .is_some_and(|p| p.iter().all(|b| matches!(b, b'/' | b'\\')))
        || (path.contains(':') && (!drive_absolute(&path) || path[2..].contains(':')))
    {
        return None;
    }
    let extension = path
        .rsplit(['/', '\\'])
        .next()?
        .rsplit_once('.')
        .filter(|(stem, ext)| {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 12
                && ext.bytes().all(|c| c.is_ascii_alphanumeric())
        })
        .map(|(_, ext)| ext);
    let has_extension = extension.is_some();
    let known_bare_filename = extension.is_some_and(|ext| {
        [
            "cs", "txt", "log", "md", "markdown", "json", "jsonc", "xml", "yaml", "yml", "asmdef",
            "asmref", "unity", "asset", "prefab", "meta", "png", "jpg", "jpeg", "gif", "svg",
            "tga",
        ]
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known))
    });
    if !markdown
        && !(path.starts_with(['/', '\\'])
            || path.starts_with("./")
            || path.starts_with("../")
            || drive_absolute(&path)
            || known_bare_filename
            || (has_extension && path.contains(['/', '\\']))
            || (position.is_some() && (has_extension || path.contains(['/', '\\'])))
            || lower.starts_with("file://"))
    {
        return None;
    }
    if path.is_empty() && fragment.is_none() {
        return None;
    }
    Some(LinkTarget::Path {
        display: value.to_owned(),
        path: path.into(),
        position,
        fragment,
        document_relative: markdown,
    })
}

fn drive_absolute(path: &str) -> bool {
    matches!(path.as_bytes(), [letter, b':', b'/' | b'\\', ..] if letter.is_ascii_alphabetic())
}

fn split_position(path: &str) -> (&str, Option<Position>) {
    let parse = |l: &str, c: &str| {
        Some(Position {
            line: Line(l.parse::<u64>().ok()?.checked_sub(1)?),
            column: rove_base::Utf8Column(c.parse::<u64>().ok()?.checked_sub(1)?),
        })
    };
    if let Some((p, suffix)) = path.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
        if let Some(position) = suffix
            .split_once(',')
            .and_then(|(l, c)| parse(l.trim(), c.trim()))
        {
            return (p, Some(position));
        }
    }
    if let Some((p, last)) = path.rsplit_once(':') {
        if let Some((base, line)) = p.rsplit_once(':') {
            if let Some(position) = parse(line, last) {
                return (base, Some(position));
            }
        }
        if let Some(position) = parse(last, "1") {
            return (p, Some(position));
        }
    }
    (path, None)
}

use gpui::Context;

use super::{cache::Lru, view::CodeView};

const MARGIN: u64 = 8;

pub(super) struct LinkCache {
    active: HashMap<u64, Vec<Link>>,
    history: Lru<u64, Vec<Link>>,
    /// Requested document lines in order, including the visible margin.
    wanted_lines: Vec<u64>,
    wanted: Option<Range<u64>>,
    pending: HashSet<u64>,
    epoch: u64,
}

impl Default for LinkCache {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
            history: Lru::new(1024 * 1024),
            wanted_lines: Vec::new(),
            wanted: None,
            pending: HashSet::new(),
            epoch: 0,
        }
    }
}

impl LinkCache {
    pub fn reset(&mut self) {
        self.active.clear();
        self.history.clear();
        self.wanted_lines.clear();
        self.invalidate_pending();
    }

    pub fn revisit_viewport(&mut self) {
        self.wanted = None;
    }

    fn invalidate_pending(&mut self) {
        self.wanted = None;
        self.pending.clear();
        self.epoch += 1;
    }

    pub fn remap(&mut self, mut map: impl FnMut(&u64) -> Option<u64>) {
        self.history.rekey(&mut map);
        self.active = std::mem::take(&mut self.active)
            .into_iter()
            .filter_map(|(line, links)| Some((map(&line)?, links)))
            .collect();
        self.invalidate_pending();
    }

    fn set_wanted_lines(&mut self, lines: Vec<u64>) {
        for (line, links) in self
            .active
            .extract_if(|line, _| lines.binary_search(line).is_err())
        {
            Self::remember(&mut self.history, line, links);
        }
        self.wanted_lines = lines;
    }

    fn insert(&mut self, line: u64, links: Vec<Link>) {
        self.pending.remove(&line);
        if self.wanted_lines.binary_search(&line).is_ok() {
            self.active.insert(line, links);
        } else {
            Self::remember(&mut self.history, line, links);
        }
    }

    fn remember(history: &mut Lru<u64, Vec<Link>>, line: u64, links: Vec<Link>) {
        let cost = 64
            + links.len() * 128
            + links
                .iter()
                .map(|l| l.target.display().len())
                .sum::<usize>();
        history.insert(line, links, cost);
    }
}

impl CodeView {
    #[cfg(test)]
    pub(crate) fn cached_link_lines(&self) -> usize {
        self.links.active.len() + self.links.history.stats().entries
    }

    pub(crate) fn link_at(&self, position: Position) -> Option<Link> {
        if !self.options().links || self.effective_level() == rove_buffer::SizeClass::Huge {
            return None;
        }
        let column = u32::try_from(position.column.0).ok()?;
        self.links
            .active
            .get(&position.line.0)
            .or_else(|| self.links.history.peek(&position.line.0))?
            .iter()
            .find(|link| link.range.contains(&column))
            .cloned()
    }

    pub(super) fn queue_visible_links(&mut self, visible: Range<u64>, cx: &mut Context<Self>) {
        if !self.options().links || self.effective_level() == rove_buffer::SizeClass::Huge {
            self.links.wanted = None;
            return;
        }
        let Some(text) = self.text().cloned() else {
            return;
        };
        let wanted = visible.start.saturating_sub(MARGIN)
            ..visible.end.saturating_add(MARGIN).min(self.line_count());
        if self.links.wanted.as_ref() == Some(&wanted) {
            return;
        }
        self.links.wanted = Some(wanted.clone());
        let mut documents: Vec<_> = wanted.map(|line| self.doc_line(line)).collect();
        documents.dedup();
        self.links.set_wanted_lines(documents);
        let mut lines = Vec::new();
        for &document in &self.links.wanted_lines {
            if !self.links.active.contains_key(&document) {
                if let Some(links) = self.links.history.remove(&document) {
                    self.links.active.insert(document, links);
                } else if self.links.pending.insert(document) {
                    lines.push(document);
                }
            }
        }
        if lines.is_empty() {
            return;
        }
        let limit = self.render_limit().unwrap_or(10_000);
        let markdown = self.language.as_deref() == Some("markdown");
        let epoch = self.links.epoch;
        let job = cx.background_executor().spawn(async move {
            lines
                .into_iter()
                .map(|line| {
                    let prefix = line_prefix(&*text, line, limit);
                    (line, scan(&prefix, markdown))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |view, cx| {
                if view.links.epoch != epoch {
                    return;
                }
                for (line, links) in result {
                    view.links.insert(line, links);
                }
                view.refresh_link_pointer(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_current_viewport_cannot_be_evicted_by_history_budget() {
        let text = "www.example.test ".repeat(500);
        let dense = scan(&text, false);
        let mut cache = LinkCache::default();
        cache.set_wanted_lines((0..25).collect());
        for line in 0..25 {
            cache.insert(line, dense.clone());
        }
        assert_eq!(cache.active.len(), 25);
        assert_eq!(cache.active.get(&0).unwrap().len(), 500);
        cache.set_wanted_lines((22..25).collect());
        assert_eq!(cache.active.len(), 3);
        assert!(cache.history.stats().bytes <= cache.history.stats().capacity);
    }

    #[test]
    fn line_limit_omits_a_partially_truncated_target() {
        let text = rove_buffer::RopeText::from_text("中 https://example.test/message");
        let prefix = line_prefix(&text, 0, 16);
        assert_eq!(prefix, "中 ");
        assert!(scan(&prefix, false).is_empty());
    }

    #[test]
    fn dense_ten_thousand_character_line_detection_measurement() {
        let part = "https://example.test/a(b) Assets/Hero.cs:41:7 ";
        let mut line = part.repeat(10_000 / part.len());
        line.push_str(&" ".repeat(10_000 - line.len()));
        let mut durations = Vec::new();
        for _ in 0..101 {
            let start = std::time::Instant::now();
            let links = std::hint::black_box(scan(std::hint::black_box(&line), false));
            durations.push(start.elapsed());
            assert_eq!(links.len(), 2 * (10_000 / part.len()));
        }
        durations.sort_unstable();
        eprintln!(
            "10000-byte dense link scan median {:?}, p99 {:?}",
            durations[50], durations[99]
        );
    }

    #[test]
    fn scans_web_and_local_file_urls_with_balanced_trailing_punctuation() {
        let text = "见 (https://example.com/a(b)). http://example.test/a, www.example.test/x; file:///C:/project/Player.cs";
        let links = scan(text, false);
        let rendered: Vec<_> = links
            .iter()
            .map(|l| &text[l.range.start as usize..l.range.end as usize])
            .collect();
        assert_eq!(
            rendered,
            [
                "https://example.com/a(b)",
                "http://example.test/a",
                "www.example.test/x",
                "file:///C:/project/Player.cs"
            ]
        );
        assert!(matches!(&links[3].target, LinkTarget::Path { .. }));
    }

    #[test]
    fn scans_a_quoted_windows_path_with_spaces_as_a_single_link() {
        let links = scan(r#"at "C:\My Project\New File.cs:41:7" now"#, false);
        assert_eq!(links.len(), 1);
        assert!(
            matches!(&links[0].target, LinkTarget::Path { path, position: Some(p), .. }
            if path == &PathBuf::from(r"C:\My Project\New File.cs") && p.line.0 == 40 && p.column.0 == 6)
        );
    }

    #[test]
    fn parses_three_compiler_link_positions_without_io() {
        let links = scan(
            r"C:\project\Player.cs:41 Assets/Player.cs:41:7 Assets/Player.cs(41,7)",
            false,
        );
        let locations: Vec<_> = links
            .iter()
            .filter_map(|link| match link.target {
                LinkTarget::Path { position, .. } => position,
                _ => None,
            })
            .collect();
        assert_eq!(locations.len(), 3);
        assert_eq!(
            locations
                .iter()
                .map(|position| (position.line.0, position.column.0))
                .collect::<Vec<_>>(),
            [(40, 0), (40, 6), (40, 6)]
        );
    }

    #[test]
    fn recognizes_markdown_inline_angle_and_reference_links() {
        let links = scan(
            "[Guide](./guide.md#section) <https://example.test/a> [Guide][manual]",
            true,
        );
        assert_eq!(links.len(), 3);
        assert!(
            matches!(&links[0].target, LinkTarget::Path { fragment: Some(anchor), .. } if anchor == "section")
        );
        assert!(matches!(&links[2].target, LinkTarget::Reference(id) if id == "manual"));
    }

    #[test]
    fn does_not_treat_network_paths_or_custom_protocols_as_local_files() {
        for value in [
            "file://server/share/a.cs",
            r"\\server\share\a.cs",
            "file:///%5c%5cserver/share/a.cs",
            "javascript:run",
            "https://user@example.test",
        ] {
            assert!(target(value, true).is_none(), "{value}");
        }
    }
}
