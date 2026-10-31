// Remplace INTÉGRALEMENT la fonction `paint_browser_chrome` de src/gui.rs
// (de `unsafe fn paint_browser_chrome` jusqu'à son `}` de fin, juste avant
// `unsafe fn paint_home_screen`). Les coordonnées sont identiques à l'ancienne
// version : le hit-testing de WM_LBUTTONDOWN ne change pas.

// À ajouter avec les autres constantes en haut de gui.rs :
// const DARK_ON_ACCENT: u32 = 0x000A1311; // #11130A en COLORREF (BGR)

unsafe fn paint_browser_chrome(state: &AppState, hdc: Handle, width: i32) {
    // Cadre technique : bande d'onglets sombre, onglet actif fondu dans la barre
    // d'outils avec un filet d'accent, onglets numérotés, angles vifs.
    let strip = chrome_color(state, |colors| &colors.toolbar_background, 0x000F0D0C);
    let window = chrome_color(state, |colors| &colors.window_background, 0x00221D1B);
    let active_tab = chrome_color(state, |colors| &colors.tab_active_background, 0x00221D1B);
    let inactive_tab = chrome_color(state, |colors| &colors.tab_inactive_background, 0x00302926);
    let accent = chrome_color(state, |colors| &colors.accent, 0x004EF2C6);
    let text = chrome_color(state, |colors| &colors.text, 0x00F4F2F1);
    let muted = chrome_color(state, |colors| &colors.muted_text, 0x00B5ABA7);
    let address = chrome_color(state, |colors| &colors.address_background, 0x000F0D0C);
    let border = chrome_color(state, |colors| &colors.border, 0x00423835);

    paint_fill(hdc, 0, 0, width, 88, strip);
    paint_fill(hdc, 0, 39, width, 90, window);
    paint_fill(hdc, 0, 86, width, 89, border);

    let tab_area = (width - 134).max(0);
    let tab_width = (tab_area / state.tabs.len().max(1) as i32).clamp(120, 220);
    let active = state.browser.active_tab_id();
    for (index, (id, _)) in state.tabs.iter().enumerate() {
        let left = 10 + index as i32 * tab_width;
        let right = (left + tab_width - 6).min(width - 68);
        if right <= left {
            continue;
        }
        let selected = Some(*id) == active;
        if selected {
            // Fondu avec la barre d'outils (même couleur) jusqu'à y = 39.
            paint_fill(hdc, left, 7, right, 39, active_tab);
            paint_fill(hdc, left, 7, right, 9, accent);
        } else {
            paint_fill(hdc, left, 11, right, 36, inactive_tab);
        }
        let title = state
            .browser
            .tabs()
            .tab(*id)
            .and_then(|tab| tab.page())
            .and_then(|page| page.title())
            .unwrap_or_else(|| "Nouvel onglet".to_owned());
        let label = title.chars().take(22).collect::<String>();
        let number = format!("{:02}", index + 1);
        paint_label(
            hdc,
            &number,
            left + 10,
            9,
            left + 30,
            36,
            if selected { accent } else { muted },
            0,
        );
        paint_label(
            hdc,
            &label,
            left + 32,
            9,
            right - if state.tabs.len() > 1 { 34 } else { 10 },
            36,
            if selected { text } else { muted },
            0,
        );
        if state.tabs.len() > 1 {
            paint_label(hdc, "×", right - 30, 9, right - 6, 36, muted, DT_CENTER);
        }
    }
    paint_round_fill(hdc, width - 108, 7, width - 62, 36, inactive_tab, 3);
    paint_label(hdc, "⋯", width - 108, 7, width - 62, 36, text, DT_CENTER);
    paint_round_fill(hdc, width - 58, 7, width - 10, 36, inactive_tab, 3);
    paint_label(hdc, "+", width - 58, 7, width - 10, 36, text, DT_CENTER);

    let can_go_back = state.browser.history().can_go_back();
    let can_go_forward = state.browser.history().can_go_forward();
    for (label, left, right, enabled) in [
        ("‹", 10, 44, can_go_back),
        ("›", 48, 82, can_go_forward),
        ("⟳", 86, 122, true),
    ] {
        paint_round_fill(
            hdc,
            left,
            48,
            right,
            80,
            if enabled { inactive_tab } else { window },
            3,
        );
        paint_label(
            hdc,
            label,
            left,
            48,
            right,
            80,
            if enabled { text } else { muted },
            DT_CENTER,
        );
    }

    // Champ d'adresse : liseré 1 px puis fond.
    let address_right = (width - 170).max(240);
    paint_round_fill(hdc, 132, 48, address_right, 80, border, 3);
    paint_round_fill(hdc, 133, 49, address_right - 1, 79, address, 3);

    let current_url = state.browser.current_url().map(ToString::to_string);
    let bookmarked = current_url
        .as_ref()
        .is_some_and(|url| state.bookmarks.iter().any(|bookmark| &bookmark.url == url));
    paint_round_fill(hdc, width - 166, 48, width - 132, 80, inactive_tab, 3);
    paint_label(
        hdc,
        if bookmarked { "★" } else { "☆" },
        width - 166,
        48,
        width - 132,
        80,
        if bookmarked { accent } else { text },
        DT_CENTER,
    );
    paint_round_fill(hdc, width - 122, 48, width - 10, 80, accent, 3);
    paint_label(
        hdc,
        "Aller",
        width - 122,
        48,
        width - 10,
        80,
        DARK_ON_ACCENT,
        DT_CENTER,
    );

    // Barre de favoris, de la même couleur que la barre d'outils.
    paint_fill(hdc, 0, 90, width, 119, window);
    paint_fill(hdc, 0, 118, width, 119, border);
    if state.bookmarks.is_empty() {
        paint_label(
            hdc,
            "Ajoutez vos pages favorites avec ☆",
            14,
            91,
            width - 14,
            117,
            muted,
            0,
        );
    } else {
        let item_width = 154;
        let visible = ((width - 24).max(0) / (item_width + 8)) as usize;
        for (index, bookmark) in state.bookmarks.iter().take(visible).enumerate() {
            let left = 12 + index as i32 * (item_width + 8);
            paint_round_fill(hdc, left, 93, left + item_width, 115, inactive_tab, 3);
            let label = bookmark.title.chars().take(20).collect::<String>();
            paint_label(
                hdc,
                &label,
                left + 8,
                93,
                left + item_width - 8,
                115,
                text,
                0,
            );
        }
    }
}
