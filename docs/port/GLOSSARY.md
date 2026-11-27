# Glossary

The original's terms and their names in the Rust port.

| Term | Meaning | In Rust |
| --- | --- | --- |
| item | anything on a tile: a game, app, service or system action | `library::Item` |
| source | where an item came from: steam, recomp, start, web, system | `Item::source` |
| category | section an item sits in: games, media, apps, system, hidden | `Item::category` |
| override | the person's change to an item: name, category, hidden, favorite, cover | `sofa.json` `overrides`, applied in `library::build` |
| recent | last launch time per item | `sofa.json` `recent` |
| art | capsule (poster), hero, logo, header, icon | `library::Art` |
| launch | how an item starts: url, appsfolder, exec, web, system | `library::Launch` |
| tab | top-level section: Home, Games, Media, Apps, System, Hidden | `ui::App::tab` |
| row | a titled line of tiles on Home | `ui::Row` |
| tile | one card on a row or grid; also the "All …" nav tile | `ui::Tile` |
| hero | the large art and title at the top for the focused tile | `ui::App::draw_hero` |
| layer | anything on top of the shelf: sheet, dialog, launch overlay | `ui::Layer` |
| sheet | right-hand panel (settings, item options) | `ui::Layer::Sheet` |
| keycap | a drawn controller button in hints | `ui::App::keycap` |
| action | what a button or key means: accept, back, menu, options… | `input::Action` |
| home button | PS (or Create + Options): bring Sofa to the front, go Home | `input::WM_PAD_HOME`, `Action::Home` |
| guide setting | Steam's "Guide button focuses Steam" (`Controller_CheckGuideButton`) | `src/steamguide.js` (not ported yet) |
| rem | layout unit: screen height / 67.5 (16 px at 1080p) | `ui::App::rem` |
