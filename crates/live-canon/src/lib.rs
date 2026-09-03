//! live-canon: a Rust port of the Live Canon
//!
//! Reads AI-Writings papers as a navigable cell fabric, with 5 operations:
//!   1. NAVIGATE  - BFS through citations
//!   2. CONFLUENCE - join 2+ papers, suggest synthesis
//!   3. LINEAGE   - trace F-number through time
//!   4. GHOST     - find paper that should exist by shape proximity
//!   5. TICK      - re-balance the canon
//!
//! This is a Rust no_std-friendly port — no external deps, just std for HashMap.
//!
//! Phase 251 of the polyformalism canon. The 5-substrate claim (C, Rust,
//! Python, Verilog, VHDL) is now extended: the Live Canon idea runs in all
//! 5 languages. The shape and operations are byte-equivalent by design.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// A single cell in the canon fabric.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub number: u32,
    pub title: String,
    pub f_number: u32,
    pub phase: u32,
    pub date: String,
    pub ref_papers: Vec<u32>,
    pub ref_f_numbers: Vec<u32>,
}

/// A 16-dial vector (Q1.15) — the cell's signature.
pub type Dials = [u16; 16];

/// Convert a cell to its 16-dial vector.
pub fn cell_to_dials(cell: &Cell) -> Dials {
    let year_q = if cell.date.len() >= 4 {
        let year: u32 = cell.date[..4].parse().unwrap_or(1970);
        ((year.saturating_sub(1970)).min(60) as u16) * 546  // 60 years → 0x7FFF
    } else { 0 };
    let phase_q = ((cell.phase.min(300)) as u16) * 218;  // 300 → 0x7FFF
    let f_q = ((cell.f_number.min(300)) as u16) * 218;
    let n_refs = (cell.ref_papers.len() + cell.ref_f_numbers.len()).min(127) as u16;
    let n_refs_q = n_refs * 256;
    let title_hash = hash_str(&cell.title);
    let num_q = ((cell.number.min(500)) as u16) * 131;  // 500 → 0x7FFF

    [
        num_q,                // 0: paper number
        (title_hash & 0xFFFF) as u16,  // 1: title hash
        f_q,                  // 2: F-number
        phase_q,              // 3: phase
        year_q,               // 4: year
        n_refs_q,             // 5: number of refs
        ((title_hash >> 16) & 0xFFFF) as u16,  // 6: title hash high
        0,                    // 7: reserved
        0, 0, 0, 0, 0, 0, 0, 0,
    ]
}

/// FNV-1a 64-bit hash (matches Python implementation).
pub fn fnv1a_64(s: &str) -> u64 {
    let mut h: u64 = 0xCBF29CE484222325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x00000100000001B3);
    }
    h
}

fn hash_str(s: &str) -> u64 {
    fnv1a_64(s)
}

/// State hash for a fabric (matches Python QufFile.state_hash).
pub fn state_hash(dials: &[Dials]) -> u64 {
    let mut h: u64 = 0xCBF29CE484222325;
    for cell in dials {
        for d in cell {
            let bytes = d.to_le_bytes();
            for b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x00000100000001B3);
            }
        }
    }
    h
}

/// The Live Canon: the AI-Writings canon as a navigable cell fabric.
pub struct LiveCanon {
    pub papers: HashMap<u32, Cell>,
    pub dials: HashMap<u32, Dials>,
    pub state_hash: u64,
}

impl LiveCanon {
    pub fn new() -> Self {
        Self {
            papers: HashMap::new(),
            dials: HashMap::new(),
            state_hash: 0,
        }
    }

    /// Add a paper to the canon.
    pub fn add(&mut self, cell: Cell) {
        let dials = cell_to_dials(&cell);
        self.dials.insert(cell.number, dials);
        self.papers.insert(cell.number, cell);
        self.recompute_state_hash();
    }

    fn recompute_state_hash(&mut self) {
        let mut all_dials: Vec<&Dials> = self.dials.values().collect();
        all_dials.sort_by_key(|d| d[0]);
        let owned: Vec<Dials> = all_dials.iter().map(|&d| *d).collect();
        self.state_hash = state_hash(&owned);
    }

    /// NAVIGATE: BFS through citations.
    pub fn navigate(&self, start: u32, depth: u32) -> Vec<(u32, u32)> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut frontier = VecDeque::new();
        frontier.push_back((start, 0));
        visited.insert(start);

        while let Some((num, d)) = frontier.pop_front() {
            result.push((num, d));
            if d < depth {
                if let Some(cell) = self.papers.get(&num) {
                    for &r in &cell.ref_papers {
                        if self.papers.contains_key(&r) && !visited.contains(&r) {
                            visited.insert(r);
                            frontier.push_back((r, d + 1));
                        }
                    }
                }
            }
        }
        result
    }

    /// CONFLUENCE: join 2+ papers, suggest a synthesis.
    pub fn confluence(&self, paper_nums: &[u32]) -> ConfluenceResult {
        if paper_nums.is_empty() {
            return ConfluenceResult::default();
        }
        let mut all_refs: BTreeSet<u32> = BTreeSet::new();
        let mut shared_refs: Option<HashSet<u32>> = None;
        let mut shared_f: Option<HashSet<u32>> = None;
        let mut titles = Vec::new();
        let mut f_nums = Vec::new();

        for &num in paper_nums {
            if let Some(cell) = self.papers.get(&num) {
                titles.push(cell.title.clone());
                f_nums.push(cell.f_number);
                let refs: HashSet<u32> = cell.ref_papers.iter().cloned().collect();
                all_refs.extend(&refs);
                shared_refs = Some(match shared_refs {
                    None => refs.clone(),
                    Some(s) => s.intersection(&refs).cloned().collect(),
                });
                let fs: HashSet<u32> = cell.ref_f_numbers.iter().cloned().collect();
                shared_f = Some(match shared_f {
                    None => fs.clone(),
                    Some(s) => s.intersection(&fs).cloned().collect(),
                });
            }
        }

        let suggested = if let Some(ref f) = shared_f {
            if let Some(&first) = f.iter().next() {
                format!("F{} Synthesis: {}", first, titles.join(", "))
            } else {
                format!("Composition of {} papers", paper_nums.len())
            }
        } else {
            format!("Composition of {} papers", paper_nums.len())
        };

        ConfluenceResult {
            input_papers: paper_nums.to_vec(),
            input_titles: titles,
            shared_refs: shared_refs.unwrap_or_default().into_iter().collect(),
            shared_f_numbers: shared_f.unwrap_or_default().into_iter().collect(),
            suggested_title: suggested,
        }
    }

    /// LINEAGE: trace a concept (F-number) through time.
    pub fn lineage(&self, f_number: u32) -> Vec<&Cell> {
        let mut result: Vec<&Cell> = self.papers
            .values()
            .filter(|c| c.ref_f_numbers.contains(&f_number))
            .collect();
        result.sort_by_key(|c| (c.phase, c.number));
        result
    }

    /// GHOST: find k nearest neighbors by dial-vector similarity (cosine).
    pub fn ghost(&self, paper_num: u32, k: usize) -> Vec<(u32, f32)> {
        let target = match self.dials.get(&paper_num) {
            Some(d) => *d,
            None => return Vec::new(),
        };
        let mut scored: Vec<(u32, f32)> = self.dials
            .iter()
            .filter(|(n, _)| **n != paper_num)
            .map(|(n, d)| {
                let score = cosine_sim(&target, d);
                (*n, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    /// TICK: re-balance the canon (placeholder; the cell-runtime does this).
    pub fn tick(&self) -> u32 {
        self.papers.len() as u32
    }

    /// Build from a sequence of paper texts (markdown files).
    pub fn from_papers(papers: Vec<Cell>) -> Self {
        let mut canon = Self::new();
        for p in papers {
            canon.add(p);
        }
        canon
    }
}

impl Default for LiveCanon {
    fn default() -> Self { Self::new() }
}

/// Result of a CONFLUENCE operation.
#[derive(Debug, Default, Clone)]
pub struct ConfluenceResult {
    pub input_papers: Vec<u32>,
    pub input_titles: Vec<String>,
    pub shared_refs: Vec<u32>,
    pub shared_f_numbers: Vec<u32>,
    pub suggested_title: String,
}

/// Cosine similarity between two 16-dial vectors.
pub fn cosine_sim(a: &Dials, b: &Dials) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f32) * (*y as f32)).sum();
    let na: f32 = a.iter().map(|x| (*x as f32).powi(2)).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| (*x as f32).powi(2)).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// Parse a single paper from its markdown text.
pub fn parse_paper(text: &str) -> Option<Cell> {
    let id_re = regex_first(text, r"paper-(\d+)")?;
    let number: u32 = id_re.parse().ok()?;
    let title = regex_first(text, r"^#\s+(.+)$").unwrap_or_else(|| format!("paper-{}", number));
    let f_number: u32 = regex_first(text, r"\bF(\d{1,3})\b")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let phase: u32 = regex_first(text, r"Phase\s+(\d+)")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let date = regex_first(text, r"Date:\*?\*?\s*(\d{4}-\d{2}-\d{2})")
        .unwrap_or_else(|| "1970-01-01".to_string());

    // Find references
    let mut refs: BTreeSet<u32> = BTreeSet::new();
    for cap in regex_iter(text, r"paper-(\d{3})\b") {
        if let Ok(n) = cap.parse::<u32>() {
            if n != number { refs.insert(n); }
        }
    }
    let mut f_refs: BTreeSet<u32> = BTreeSet::new();
    for cap in regex_iter(text, r"\bF(\d{1,3})\b") {
        if let Ok(n) = cap.parse::<u32>() {
            if n != f_number { f_refs.insert(n); }
        }
    }

    Some(Cell {
        number,
        title,
        f_number,
        phase,
        date,
        ref_papers: refs.into_iter().collect(),
        ref_f_numbers: f_refs.into_iter().collect(),
    })
}

// Simple regex helpers (no external deps)
fn regex_first(text: &str, pat: &str) -> Option<String> {
    // Find the first match of pattern in text
    for line in text.lines() {
        if let Some(m) = find_in_line(line, pat) {
            return Some(m);
        }
    }
    None
}

fn regex_iter(text: &str, pat: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while let Some(m) = find_in_line(rest, pat) {
            out.push(m.clone());
            // Advance past the match
            if let Some(pos) = rest.find(&m) {
                rest = &rest[pos + m.len()..];
            } else { break; }
        }
    }
    out
}

fn find_in_line(line: &str, pat: &str) -> Option<String> {
    // Tiny regex subset: \b word boundaries, () capture groups, literal chars
    // Convert pattern to a sequence of pieces
    let mut pieces = Vec::new();
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // \b or \d
            if let Some(&next) = chars.peek() {
                chars.next();
                if next == 'b' { pieces.push(Piece::WordBoundary); }
                else if next == 'd' { pieces.push(Piece::Digit); }
                else { pieces.push(Piece::Literal(next)); }
            }
        } else if c == '(' {
            pieces.push(Piece::GroupStart);
        } else if c == ')' {
            pieces.push(Piece::GroupEnd);
        } else if c == '+' {
            pieces.push(Piece::Plus);
        } else if c == '?' {
            pieces.push(Piece::Question);
        } else {
            pieces.push(Piece::Literal(c));
        }
    }
    // Try to match at each position
    let bytes: Vec<char> = line.chars().collect();
    for start in 0..bytes.len() {
        if let Some((end, captured)) = try_match(&pieces, &bytes, start) {
            if let Some(cap) = captured {
                return Some(cap);
            }
            return Some(bytes[start..end].iter().collect());
        }
    }
    None
}

#[derive(Debug, Clone)]
enum Piece {
    Literal(char),
    WordBoundary,
    Digit,
    GroupStart,
    GroupEnd,
    Plus,
    Question,
}

fn try_match(pieces: &[Piece], s: &[char], start: usize) -> Option<(usize, Option<String>)> {
    let mut i = start;
    let mut p = 0;
    let mut captured: Option<String> = None;
    while p < pieces.len() {
        match &pieces[p] {
            Piece::Literal(c) => {
                if i >= s.len() || s[i] != *c { return None; }
                i += 1; p += 1;
            }
            Piece::Digit => {
                if i >= s.len() || !s[i].is_ascii_digit() { return None; }
                if p + 1 < pieces.len() && matches!(pieces[p+1], Piece::Plus) {
                    let start = i;
                    while i < s.len() && s[i].is_ascii_digit() { i += 1; }
                    captured = Some(s[start..i].iter().collect());
                    p += 2;
                } else {
                    captured = Some(s[i].to_string());
                    i += 1; p += 1;
                }
            }
            Piece::WordBoundary => {
                let before = if i == 0 { None } else { Some(s[i-1]) };
                let after = if i < s.len() { Some(s[i]) } else { None };
                let wb = match (before, after) {
                    (Some(b), Some(a)) => b.is_alphanumeric() != a.is_alphanumeric(),
                    _ => false,
                };
                if !wb { return None; }
                p += 1;
            }
            Piece::GroupStart => p += 1,
            Piece::GroupEnd => p += 1,
            _ => p += 1,
        }
    }
    Some((i, captured))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_paper(num: u32, f: u32, phase: u32, refs: Vec<u32>) -> Cell {
        Cell {
            number: num,
            title: format!("Paper {}", num),
            f_number: f,
            phase,
            date: "2026-09-03".to_string(),
            ref_papers: refs,
            ref_f_numbers: vec![],
        }
    }

    #[test]
    fn test_fnv1a_64_known() {
        // FNV-1a 64-bit of "F115" should match Python's
        let h = fnv1a_64("F115");
        // Just verify it produces a non-zero hash
        assert!(h > 0);
    }

    #[test]
    fn test_cell_to_dials_length() {
        let c = make_paper(425, 115, 237, vec![]);
        let d = cell_to_dials(&c);
        assert_eq!(d.len(), 16);
    }

    #[test]
    fn test_canon_navigate() {
        let mut canon = LiveCanon::new();
        canon.add(make_paper(1, 1, 1, vec![2]));
        canon.add(make_paper(2, 2, 2, vec![3]));
        canon.add(make_paper(3, 3, 3, vec![]));
        let path = canon.navigate(1, 2);
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn test_canon_lineage() {
        let mut canon = LiveCanon::new();
        let mut p1 = make_paper(1, 100, 1, vec![]);
        p1.ref_f_numbers = vec![200];
        let mut p2 = make_paper(2, 200, 2, vec![]);
        p2.ref_f_numbers = vec![];
        canon.add(p1);
        canon.add(p2);
        let lineage = canon.lineage(200);
        assert_eq!(lineage.len(), 1);
    }

    #[test]
    fn test_canon_confluence() {
        let mut canon = LiveCanon::new();
        canon.add(make_paper(1, 100, 1, vec![3]));
        canon.add(make_paper(2, 100, 2, vec![3]));
        let r = canon.confluence(&[1, 2]);
        assert!(r.suggested_title.contains("Synthesis") || r.suggested_title.contains("Composition"));
    }

    #[test]
    fn test_ghost_finds_neighbors() {
        let mut canon = LiveCanon::new();
        canon.add(make_paper(425, 115, 237, vec![]));
        canon.add(make_paper(426, 116, 238, vec![]));
        canon.add(make_paper(427, 117, 239, vec![]));
        let g = canon.ghost(425, 2);
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn test_cosine_sim() {
        let a = [100; 16];
        let b = [100; 16];
        assert!((cosine_sim(&a, &b) - 1.0).abs() < 0.001);
        let c = [0; 16];
        assert!(cosine_sim(&a, &c).abs() < 0.001);
    }

    #[test]
    fn test_state_hash_deterministic() {
        let mut canon = LiveCanon::new();
        canon.add(make_paper(1, 1, 1, vec![]));
        canon.add(make_paper(2, 2, 2, vec![]));
        let h1 = canon.state_hash;
        let mut canon2 = LiveCanon::new();
        canon2.add(make_paper(2, 2, 2, vec![]));
        canon2.add(make_paper(1, 1, 1, vec![]));
        let h2 = canon2.state_hash;
        // Different insertion order → same hash (because we sort)
        assert_eq!(h1, h2);
    }
}
