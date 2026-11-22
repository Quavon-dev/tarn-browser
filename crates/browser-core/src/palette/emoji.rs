use crate::command::fold;

/// The most rows one emoji search lists.
pub const MAX_ROWS: usize = 6;

/// Emoji by name, the first word the one rows show; written as escapes (hard rule 19).
const EMOJI: [(&str, &str); 150] = [
    ("\u{1F600}", "grinning smile happy"),
    ("\u{1F604}", "smile happy joy"),
    ("\u{1F601}", "grin beaming"),
    ("\u{1F606}", "laughing satisfied"),
    ("\u{1F605}", "sweat smile relief"),
    ("\u{1F602}", "joy tears laugh lol"),
    ("\u{1F923}", "rofl rolling laugh"),
    ("\u{1F642}", "slight smile"),
    ("\u{1F643}", "upside down"),
    ("\u{1F609}", "wink"),
    ("\u{1F60A}", "blush smile"),
    ("\u{1F607}", "innocent halo angel"),
    ("\u{1F970}", "adoring hearts love"),
    ("\u{1F60D}", "heart eyes love"),
    ("\u{1F929}", "starstruck"),
    ("\u{1F618}", "kiss heart"),
    ("\u{1F60B}", "yum delicious"),
    ("\u{1F61C}", "tongue wink"),
    ("\u{1F914}", "thinking hmm"),
    ("\u{1F928}", "eyebrow raised"),
    ("\u{1F610}", "neutral"),
    ("\u{1F644}", "eyeroll"),
    ("\u{1F60F}", "smirk"),
    ("\u{1F62C}", "grimace"),
    ("\u{1F60C}", "relieved"),
    ("\u{1F614}", "pensive"),
    ("\u{1F634}", "sleeping zzz"),
    ("\u{1F637}", "mask sick"),
    ("\u{1F922}", "nauseated sick"),
    ("\u{1F92F}", "mindblown exploding head"),
    ("\u{1F973}", "party partying celebrate"),
    ("\u{1F60E}", "sunglasses cool"),
    ("\u{1F913}", "nerd"),
    ("\u{1F615}", "confused"),
    ("\u{1F61F}", "worried"),
    ("\u{1F62E}", "surprised open mouth"),
    ("\u{1F632}", "astonished shocked"),
    ("\u{1F633}", "flushed"),
    ("\u{1F97A}", "pleading puppy eyes"),
    ("\u{1F622}", "cry sad tear"),
    ("\u{1F62D}", "sob crying"),
    ("\u{1F631}", "scream fear"),
    ("\u{1F629}", "weary tired"),
    ("\u{1F621}", "rage angry pout"),
    ("\u{1F620}", "angry mad"),
    ("\u{1F608}", "devil imp"),
    ("\u{1F480}", "skull dead"),
    ("\u{1F4A9}", "poop"),
    ("\u{1F921}", "clown"),
    ("\u{1F47B}", "ghost"),
    ("\u{1F47D}", "alien"),
    ("\u{1F916}", "robot bot"),
    ("\u{1F648}", "monkey see no evil"),
    ("\u{1F48B}", "kiss lips"),
    ("\u{1F496}", "sparkling heart"),
    ("\u{1F494}", "broken heart"),
    ("\u{1F9E1}", "orange heart"),
    ("\u{1F49B}", "yellow heart"),
    ("\u{1F49A}", "green heart"),
    ("\u{1F499}", "blue heart"),
    ("\u{1F49C}", "purple heart"),
    ("\u{1F5A4}", "black heart"),
    ("\u{1F4AF}", "hundred 100 perfect"),
    ("\u{1F4A5}", "boom collision"),
    ("\u{1F4AB}", "dizzy"),
    ("\u{1F4AC}", "speech bubble"),
    ("\u{1F44B}", "wave hello hi"),
    ("\u{1F44C}", "ok hand"),
    ("\u{1F91E}", "fingers crossed luck"),
    ("\u{1F918}", "horns rock"),
    ("\u{1F448}", "left point"),
    ("\u{1F449}", "right point"),
    ("\u{1F446}", "up point"),
    ("\u{1F447}", "down point"),
    ("\u{1F44D}", "thumbsup like yes"),
    ("\u{1F44E}", "thumbsdown dislike no"),
    ("\u{1F44A}", "punch fist bump"),
    ("\u{1F44F}", "clap applause"),
    ("\u{1F64C}", "hooray raised hands"),
    ("\u{1F64F}", "pray please thanks"),
    ("\u{1F4AA}", "muscle strong flex"),
    ("\u{1F9E0}", "brain"),
    ("\u{1F440}", "eyes look"),
    ("\u{1F937}", "shrug"),
    ("\u{1F926}", "facepalm"),
    ("\u{1F436}", "dog puppy"),
    ("\u{1F431}", "cat kitten"),
    ("\u{1F42D}", "mouse"),
    ("\u{1F430}", "rabbit bunny"),
    ("\u{1F98A}", "fox"),
    ("\u{1F43B}", "bear"),
    ("\u{1F43C}", "panda"),
    ("\u{1F981}", "lion"),
    ("\u{1F42E}", "cow"),
    ("\u{1F437}", "pig"),
    ("\u{1F438}", "frog"),
    ("\u{1F427}", "penguin"),
    ("\u{1F426}", "bird"),
    ("\u{1F989}", "owl"),
    ("\u{1F984}", "unicorn"),
    ("\u{1F41D}", "bee"),
    ("\u{1F98B}", "butterfly"),
    ("\u{1F422}", "turtle"),
    ("\u{1F419}", "octopus"),
    ("\u{1F433}", "whale"),
    ("\u{1F338}", "blossom cherry flower"),
    ("\u{1F339}", "rose flower"),
    ("\u{1F33B}", "sunflower"),
    ("\u{1F333}", "tree"),
    ("\u{1F335}", "cactus"),
    ("\u{1F340}", "clover luck"),
    ("\u{1F30D}", "earth globe world"),
    ("\u{1F319}", "moon night"),
    ("\u{1F31F}", "star glowing"),
    ("\u{1F525}", "fire hot lit"),
    ("\u{1F308}", "rainbow"),
    ("\u{1F30A}", "ocean wave water"),
    ("\u{1F34E}", "apple"),
    ("\u{1F34C}", "banana"),
    ("\u{1F353}", "strawberry"),
    ("\u{1F951}", "avocado"),
    ("\u{1F35E}", "bread"),
    ("\u{1F354}", "burger hamburger"),
    ("\u{1F355}", "pizza"),
    ("\u{1F32E}", "taco"),
    ("\u{1F363}", "sushi"),
    ("\u{1F369}", "doughnut donut"),
    ("\u{1F382}", "birthday cake"),
    ("\u{1F37A}", "beer"),
    ("\u{1F377}", "wine"),
    ("\u{2615}", "coffee"),
    ("\u{26BD}", "soccer football"),
    ("\u{1F3AE}", "gaming controller"),
    ("\u{1F3B5}", "music note"),
    ("\u{1F680}", "rocket launch ship"),
    ("\u{1F697}", "car"),
    ("\u{1F3E0}", "house home"),
    ("\u{1F4BB}", "laptop computer"),
    ("\u{1F4A1}", "idea bulb light"),
    ("\u{1F4B0}", "money bag"),
    ("\u{1F4E6}", "package box"),
    ("\u{1F4DD}", "memo note"),
    ("\u{1F4C5}", "calendar date"),
    ("\u{1F512}", "lock locked"),
    ("\u{1F389}", "tada party celebrate"),
    ("\u{1F3C6}", "trophy win"),
    ("\u{1F6A8}", "siren alert"),
    ("\u{2705}", "check done yes"),
    ("\u{274C}", "cross no"),
    ("\u{2728}", "sparkles shiny"),
];

/// Emoji whose name has a word starting with each word after a leading `:`, in table order, at
/// most `MAX_ROWS`: the emoji and its name.
pub fn search(text: &str) -> Vec<(&'static str, &'static str)> {
    let Some(query) = text.trim().strip_prefix(':') else { return Vec::new() };
    let query: String = query.chars().flat_map(fold).collect();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() || query.len() > 40 {
        return Vec::new();
    }
    EMOJI
        .iter()
        .filter(|(_, names)| words.iter().all(|w| names.split(' ').any(|n| n.starts_with(w))))
        .map(|(emoji, names)| (*emoji, names.split(' ').next().unwrap_or(names)))
        .take(MAX_ROWS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_emoji_by_word_starts_after_a_colon() {
        assert_eq!(search(":rocket"), [("\u{1F680}", "rocket")]);
        assert_eq!(search(" :TACO "), [("\u{1F32E}", "taco")]);
        assert_eq!(search(":heart broken"), [("\u{1F494}", "broken")]);
        assert!(search(":heart").len() > 3 && search(":heart").len() <= MAX_ROWS);
        assert!(search("rocket").is_empty(), "only after a colon");
        assert!(search(":").is_empty() && search(":zzzzz").is_empty());
    }

    #[test]
    fn every_entry_is_one_named_visible_emoji() {
        for (emoji, names) in EMOJI {
            assert_eq!(emoji.chars().count(), 1, "{names}");
            assert!(!emoji.chars().any(|c| c == '\u{fe0f}' || c == '\u{200d}'), "{names}");
            assert!(!names.is_empty() && names == names.to_lowercase(), "{names}");
        }
    }
}
