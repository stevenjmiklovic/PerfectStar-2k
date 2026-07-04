//! The startup splash screen: a big block-letter "PERFECTSTAR" banner,
//! dismissed by any keypress. Text-mode DOS programs couldn't vary font
//! size, so splash logos were drawn by hand, pixel by pixel, with a single
//! block character — this reproduces that instead of shelling out to a
//! figlet-style font engine we don't otherwise need.

const ROWS: usize = 5;

/// A letter's pixel grid: 5 rows of 5 columns, '#' filled / '.' empty.
fn glyph(c: char) -> [&'static str; ROWS] {
    match c {
        'P' => ["####.", "#...#", "####.", "#....", "#...."],
        'E' => ["#####", "#....", "###..", "#....", "#####"],
        'R' => ["####.", "#...#", "####.", "#..#.", "#...#"],
        'F' => ["#####", "#....", "###..", "#....", "#...."],
        'C' => [".####", "#....", "#....", "#....", ".####"],
        'T' => ["#####", "..#..", "..#..", "..#..", "..#.."],
        'S' => [".####", "#....", ".###.", "....#", "####."],
        'A' => [".###.", "#...#", "#####", "#...#", "#...#"],
        _ => [".....", ".....", ".....", ".....", "....."],
    }
}

/// Render `word` as a 5-row block-letter banner, one column of gap between
/// letters, '#' pixels drawn as the full-block character.
pub fn banner(word: &str) -> [String; ROWS] {
    let mut rows: [String; ROWS] = std::array::from_fn(|_| String::new());
    for (i, letter) in word.chars().enumerate() {
        if i > 0 {
            for row in rows.iter_mut() {
                row.push(' ');
            }
        }
        let g = glyph(letter);
        for (row, glyph_row) in rows.iter_mut().zip(g.iter()) {
            row.extend(glyph_row.chars().map(|px| if px == '#' { '█' } else { ' ' }));
        }
    }
    rows
}
