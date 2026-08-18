#[cfg(test)]
mod tests {
    use crate::render;
    use crate::truncate;
    use crate::{CellKind, CellRow, TuiMode, TuiState, TuiStatus};

    fn sample_state() -> TuiState {
        TuiState {
            sheet_id: "test".into(),
            cells: vec![
                CellRow {
                    id: "a".into(),
                    kind: CellKind::Value,
                    value: "10".into(),
                    status: TuiStatus::Ready,
                    error: None,
                    dependencies: vec![],
                    dependents: vec!["sum".into()],
                },
                CellRow {
                    id: "sum".into(),
                    kind: CellKind::Formula,
                    value: "30".into(),
                    status: TuiStatus::Ready,
                    error: None,
                    dependencies: vec!["a".into(), "b".into()],
                    dependents: vec![],
                },
            ],
            selected: 1,
            view_top: 0,
            viewport_height: 10,
            mode: TuiMode::Normal,
            edit_buffer: "".into(),
            status: None,
        }
    }

    #[test]
    fn render_includes_sheet_id() {
        let out = render(&sample_state());
        assert!(out.contains("test"), "missing sheet id: {}", out);
    }

    #[test]
    fn render_includes_cell_ids() {
        let out = render(&sample_state());
        assert!(out.contains("a"));
        assert!(out.contains("sum"));
    }

    #[test]
    fn render_includes_deps_for_selected() {
        let out = render(&sample_state());
        // Selected is "sum" (index 1). Should show its deps.
        assert!(out.contains("deps:"));
        assert!(out.contains("a, b"));
    }

    #[test]
    fn render_includes_dependents_for_selected() {
        // Switch selection to "a" (index 0).
        let mut s = sample_state();
        s.selected = 0;
        let out = render(&s);
        assert!(out.contains("used by:"));
        assert!(out.contains("sum"));
    }

    #[test]
    fn render_shows_keymap_in_normal_mode() {
        let out = render(&sample_state());
        assert!(out.contains("j/k=navigate"));
        assert!(out.contains("q=quit"));
    }

    #[test]
    fn render_shows_edit_prompt_in_set_mode() {
        let mut s = sample_state();
        s.mode = TuiMode::Set;
        s.edit_buffer = "42".into();
        let out = render(&s);
        assert!(out.contains("set sum = 42"));
    }

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_elided() {
        let result = truncate("a long string that should be truncated", 10);
        assert!(result.chars().count() <= 10);
        assert!(result.ends_with('…'));
    }
}
