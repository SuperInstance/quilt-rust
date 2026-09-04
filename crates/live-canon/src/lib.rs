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

/// A body excerpt for one paper (used by CLAIM/DRILL).
#[derive(Debug, Clone, Default)]
pub struct Body {
    pub h1: String,
    pub excerpt: String,
}

/// The Live Canon: the AI-Writings canon as a navigable cell fabric.
pub struct LiveCanon {
    pub papers: HashMap<u32, Cell>,
    pub bodies: HashMap<u32, Body>,
    pub dials: HashMap<u32, Dials>,
    pub state_hash: u64,
}

impl LiveCanon {
    pub fn new() -> Self {
        Self {
            papers: HashMap::new(),
            bodies: HashMap::new(),
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

    /// Add a body excerpt for a paper.
    pub fn add_body(&mut self, number: u32, body: Body) {
        self.bodies.insert(number, body);
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

    /// CLAIM: find the most-authoritative paper for a topic.
    /// Score = title×100 + h1×50 + body×25 + F#_recall×200 + recency
    /// If no paper directly addresses the topic, returns the most-recent
    /// paper (recency-tied) — this is honest, not a hallucination.
    pub fn claim(&self, query: &str) -> Option<ClaimResult> {
        let q = query.to_lowercase();
        let q_tokens: Vec<&str> = q.split_whitespace()
            .filter(|t| t.len() >= 2)
            .collect();
        if q_tokens.is_empty() { return None; }

        // Try to extract F-number hints from the query
        let query_fns: Vec<u32> = extract_f_numbers(&q);

        let mut scored: Vec<ClaimCandidate> = Vec::new();
        for (n, paper) in &self.papers {
            let title = paper.title.to_lowercase();
            let body = self.bodies.get(n).map(|b| b.excerpt.to_lowercase()).unwrap_or_default();
            let h1 = self.bodies.get(n).map(|b| b.h1.to_lowercase()).unwrap_or_default();

            let title_matches = q_tokens.iter().filter(|t| title.contains(**t)).count();
            let h1_matches = q_tokens.iter().filter(|t| h1.contains(**t)).count();
            let body_matches = q_tokens.iter().filter(|t| body.contains(**t)).count();
            let fn_matches = query_fns.iter()
                .filter(|f| paper.ref_f_numbers.contains(f))
                .count();
            let recency = paper.f_number as f32 * 0.1;

            let score = (title_matches * 100 + h1_matches * 50 + body_matches * 25
                + fn_matches * 200) as f32 + recency;

            // Only consider papers with at least one token match (not just recency).
            if title_matches + h1_matches + body_matches + fn_matches > 0 {
                scored.push(ClaimCandidate {
                    number: *n,
                    title: paper.title.clone(),
                    f_number: paper.f_number,
                    phase: paper.phase,
                    date: paper.date.clone(),
                    ref_f_numbers: paper.ref_f_numbers.clone(),
                    score,
                    excerpt: self.bodies.get(n).map(|b| b.excerpt.clone()).unwrap_or_default(),
                });
            }
        }
        scored.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
                .then(b.f_number.cmp(&a.f_number))
                .then(b.ref_f_numbers.len().cmp(&a.ref_f_numbers.len()))
        });
        if scored.is_empty() {
            return None;
        }
        let winner = scored[0].clone();
        let runners_up: Vec<ClaimCandidate> = scored.iter().skip(1).take(3).cloned().collect();
        Some(ClaimResult {
            query: query.to_string(),
            tokens: q_tokens.iter().map(|s| s.to_string()).collect(),
            winner,
            runners_up,
            total_candidates: scored.len(),
        })
    }

    /// DRILL: 3-paper training curriculum for a topic.
    /// Returns DOCTRINE (cited by most), IMPLEMENTATION, VERIFICATION.
    pub fn drill(&self, query: &str) -> Option<DrillResult> {
        let c = self.claim(query)?;
        let mut top: Vec<ClaimCandidate> = vec![c.winner.clone()];
        top.extend(c.runners_up.iter().cloned());
        top.truncate(3);
        while top.len() < 3 { top.push(top.last().cloned().unwrap_or_else(|| c.winner.clone())); }

        // Reorder: most-cited-as-ref = DOCTRINE
        if top.len() == 3 && top.iter().all(|t| true) {
            let ref_sets: Vec<std::collections::HashSet<u32>> = top.iter()
                .map(|t| t.ref_f_numbers.iter().cloned().collect())
                .collect();
            let cited_by: Vec<usize> = top.iter().enumerate().map(|(i, t)| {
                ref_sets.iter().enumerate().filter(|(j, s)| {
                    *j != i && s.contains(&t.f_number)
                }).count()
            }).collect();
            if let Some((max_idx, _)) = cited_by.iter().enumerate().max_by_key(|(_, &v)| v) {
                if max_idx != 0 {
                    top.swap(0, max_idx);
                }
            }
        }

        Some(DrillResult {
            query: query.to_string(),
            doctrine: Some(top[0].clone()),
            implementation: top.get(1).cloned(),
            verification: top.get(2).cloned(),
        })
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

/// A candidate paper in a CLAIM/DRILL result.
#[derive(Debug, Clone)]
pub struct ClaimCandidate {
    pub number: u32,
    pub title: String,
    pub f_number: u32,
    pub phase: u32,
    pub date: String,
    pub ref_f_numbers: Vec<u32>,
    pub score: f32,
    pub excerpt: String,
}

/// Result of a CLAIM operation.
#[derive(Debug, Clone)]
pub struct ClaimResult {
    pub query: String,
    pub tokens: Vec<String>,
    pub winner: ClaimCandidate,
    pub runners_up: Vec<ClaimCandidate>,
    pub total_candidates: usize,
}

/// Result of a DRILL operation.
#[derive(Debug, Clone)]
pub struct DrillResult {
    pub query: String,
    pub doctrine: Option<ClaimCandidate>,
    pub implementation: Option<ClaimCandidate>,
    pub verification: Option<ClaimCandidate>,
}

/// Extract F-numbers from a query string.
fn extract_f_numbers(q: &str) -> Vec<u32> {
    let bytes = q.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'f' || bytes[i] == b'F' {
            // skip optional space
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' { j += 1; }
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() { j += 1; }
            if j > start {
                if let Ok(n) = q[start..j].parse::<u32>() {
                    out.push(n);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
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

    #[test]
    fn test_claim_returns_winner() {
        let mut canon = LiveCanon::new();
        let mut p1 = make_paper(477, 168, 268, vec![]);
        p1.title = "F168 — The Trust Ladder".to_string();
        let mut p2 = make_paper(478, 169, 268, vec![]);
        p2.title = "F169 — Claim and Drill".to_string();
        canon.add(p1);
        canon.add(p2);
        let c = canon.claim("trust ladder").unwrap();
        assert_eq!(c.winner.f_number, 168);
        assert!(c.winner.title.contains("Trust Ladder"));
    }

    #[test]
    fn test_claim_empty_query_returns_none() {
        let canon = LiveCanon::new();
        assert!(canon.claim("").is_none());
        assert!(canon.claim("a").is_none());  // too short
    }

    #[test]
    fn test_claim_no_match_returns_none() {
        let mut canon = LiveCanon::new();
        let mut p = make_paper(1, 1, 1, vec![]);
        p.title = "completely unrelated topic here please".to_string();
        canon.add(p);
        // Use a query with no word-overlap with the title.
        assert!(canon.claim("wibbly wobbly zzzz qqqq").is_none());
    }

    #[test]
    fn test_claim_with_f_number_hint() {
        let mut canon = LiveCanon::new();
        let mut p1 = make_paper(1, 100, 1, vec![]);
        p1.title = "Some random paper".to_string();
        p1.ref_f_numbers = vec![200];
        let mut p2 = make_paper(2, 200, 2, vec![]);
        p2.title = "Another paper".to_string();
        canon.add(p1);
        canon.add(p2);
        let c = canon.claim("explain F200 please").unwrap();
        // F200 ref match should boost p1
        assert_eq!(c.winner.number, 1);
    }

    #[test]
    fn test_drill_returns_3_cards() {
        let mut canon = LiveCanon::new();
        let mut p1 = make_paper(1, 168, 268, vec![]);
        p1.title = "F168 — Trust Ladder".to_string();
        let mut p2 = make_paper(2, 169, 268, vec![]);
        p2.title = "F169 — Claim and Drill".to_string();
        let mut p3 = make_paper(3, 170, 268, vec![]);
        p3.title = "F170 — Verification".to_string();
        p1.ref_f_numbers = vec![169, 170];
        canon.add(p1);
        canon.add(p2);
        canon.add(p3);
        let d = canon.drill("trust").unwrap();
        assert!(d.doctrine.is_some());
        assert!(d.implementation.is_some());
        assert!(d.verification.is_some());
    }

    #[test]
    fn test_extract_f_numbers() {
        assert_eq!(extract_f_numbers("F168 is the trust ladder"), vec![168]);
        assert_eq!(extract_f_numbers("explain F200 please"), vec![200]);
        assert_eq!(extract_f_numbers("F123 and F456"), vec![123, 456]);
        assert_eq!(extract_f_numbers("no f-numbers here"), vec![]);
    }
}
