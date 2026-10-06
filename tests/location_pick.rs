// SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The candidate picker (step 20) driven through injected streams.
//!
//! No TTY is involved and nothing here touches the network or the filesystem: the tests feed the
//! answers the prompt accepts — an index, Enter, `q`, `Q`, junk and EOF — through a `Cursor` and
//! read the rendered list back from a `Vec<u8>`. The CLI adds the policy that decides *whether* to
//! ask, and `tests/cli_pick.rs` covers that wiring end to end.

use std::io::Cursor;

use chrono_tz::Tz;
use cirrocast::geo::pick::Picker;
use cirrocast::model::{Location, LocationSource};

/// The three ranked candidates the recorded ambiguous geocode response produces for `Beijing`,
/// winner first — the same shape `Resolved::candidates` carries.
fn candidates() -> Vec<Location> {
    vec![
        candidate(
            "Beijing",
            Some("Beijing Municipality"),
            39.9075,
            116.39723,
            Some(18_960_744),
        ),
        candidate("Beijing", Some("Shanxi"), 35.20917, 110.73278, None),
        candidate("Beijing", Some("Jiangxi"), 29.34644, 116.19873, None),
    ]
}

/// One geocoded candidate with the fields the list renders.
fn candidate(
    name: &str,
    admin1: Option<&str>,
    lat: f64,
    lon: f64,
    population: Option<u64>,
) -> Location {
    Location {
        name: name.to_owned(),
        admin1: admin1.map(str::to_owned),
        country: "China".to_owned(),
        country_code: Some("CN".to_owned()),
        lat,
        lon,
        tz: Tz::Asia__Shanghai,
        elevation_m: None,
        population,
        source: LocationSource::Geocoder,
        station: None,
        named_by: None,
    }
}

/// Runs one prompt over `answers` and returns the outcome with everything written to the output.
fn run(
    answers: &str,
    candidates: &[Location],
) -> (Result<Location, cirrocast::error::Error>, String) {
    let mut input = Cursor::new(answers.as_bytes().to_vec());
    let mut output = Vec::new();
    let chosen = {
        let mut picker = Picker::new(&mut input, &mut output);
        picker.choose("Beijing", candidates)
    };
    (
        chosen,
        String::from_utf8(output).expect("the list is UTF-8"),
    )
}

#[test]
fn the_list_marks_the_winner_and_enter_takes_it() {
    let candidates = candidates();
    let (chosen, output) = run("\n", &candidates);

    assert_eq!(chosen.expect("Enter selects"), candidates[0]);
    assert!(
        output.starts_with(
            "[1] * Beijing, Beijing Municipality, China (39.91, 116.40) Asia/Shanghai \
             (pop. 18960744)\n\
             [2]   Beijing, Shanxi, China (35.21, 110.73) Asia/Shanghai\n\
             [3]   Beijing, Jiangxi, China (29.35, 116.20) Asia/Shanghai\n"
        ),
        "{output}"
    );
    assert!(
        output.ends_with("choose a location [1-3, Enter=1, q=quit]: "),
        "{output}"
    );
}

#[test]
fn an_index_selects_that_candidate() {
    let candidates = candidates();
    for (answer, index) in [("2\n", 1), ("3\n", 2), (" 1 \n", 0)] {
        let (chosen, output) = run(answer, &candidates);
        assert_eq!(
            chosen.unwrap_or_else(|error| panic!("`{answer}` should select: {error}")),
            candidates[index],
            "`{answer}`"
        );
        assert!(output.contains("[2]  "), "{output}");
    }
}

#[test]
fn q_and_uppercase_q_give_up_with_exit_five() {
    let candidates = candidates();
    for answer in ["q\n", "Q\n", ""] {
        let (chosen, output) = run(answer, &candidates);
        let error = chosen.expect_err(&format!("`{answer}` gives up"));
        assert_eq!(error.exit_code(), 5, "`{answer}`: {error}");
        assert!(
            error
                .to_string()
                .contains("location not found: no location selected for Beijing"),
            "`{answer}`: {error}"
        );
        assert!(
            output.contains("[1] *"),
            "the list was printed first: {output}"
        );
    }
}

#[test]
fn three_invalid_answers_are_a_usage_error_naming_the_accepted_input() {
    let candidates = candidates();
    let (chosen, output) = run("9\n0\nx\n", &candidates);
    let error = chosen.expect_err("junk never selects");
    assert_eq!(error.exit_code(), 2, "{error}");
    assert!(
        error
            .to_string()
            .contains("enter a number 1..=3, Enter for 1, or q to quit"),
        "{error}"
    );
    assert!(
        output.contains("invalid input; enter a number 1..=3"),
        "{output}"
    );

    // A valid answer before the third strike still wins: only three *consecutive* invalid
    // answers give up.
    let (chosen, _) = run("9\n0\n2\n", &candidates);
    assert_eq!(chosen.expect("the third answer selects"), candidates[1]);
}

#[test]
fn the_prompt_repeats_after_an_invalid_answer() {
    let candidates = candidates();
    let (chosen, output) = run("junk\n1\n", &candidates);
    assert_eq!(chosen.expect("the second answer selects"), candidates[0]);
    assert_eq!(
        output
            .matches("choose a location [1-3, Enter=1, q=quit]: ")
            .count(),
        2,
        "{output}"
    );
}
