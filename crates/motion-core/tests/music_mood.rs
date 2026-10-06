//! (0.23 C4a) Story mood and bed selection (`audio::story_mood`,
//! `audio::select_bed`) over the labelled stories of
//! `golden/fixtures/music_mood/labels.json`.
//!
//! Run `cargo test -p motion-core --test music_mood -- --nocapture` to print
//! the per-story table (story, expected family, family read, the bed chosen
//! per tone) and the score margins.
//!
//! The Wallet Atlas intents under `brand/` are untracked; the two optional
//! labelled cases are read from the repository root, else from the directory
//! in `MOTION_MOOD_EXTRA_ROOT`, and skipped when absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use motion_core::audio::{
    select_bed, story_mood, BedChoice, MoodFamily, MoodProfile, MusicCatalog, MusicWord, TempoBand,
};
use motion_core::compiler::direction::take_seed;
use motion_core::compiler::taste::{resolve, story_key};
use motion_core::intent::CreativeIntent;
use motion_core::music_mood::{
    compatible_families, family_label, score_story, select_bed_with_tie_band,
};
use motion_core::style::StyleProfile;
use serde_json::Value;

const TONES: [&str; 9] = [
    "auto",
    "editorial",
    "technical",
    "playful",
    "street",
    "documentary",
    "hype",
    "studio",
    "cinematic",
];
const TAKES: [u64; 4] = [0, 1, 2, 3];
const SHARE_FLOOR: f64 = 0.85;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn catalog() -> MusicCatalog {
    MusicCatalog::from_json(&read("assets/music/catalog.json")).expect("catalog v0.2 parses")
}

fn family(s: &str) -> MoodFamily {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or_else(|e| panic!("{s}: {e}"))
}

struct Story {
    name: String,
    expected: MoodFamily,
    accepted: Vec<MoodFamily>,
    own_style: StyleProfile,
    has_style: bool,
    intent: CreativeIntent,
    optional: bool,
}

fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.trim_end_matches(".intent.json").to_string()
}

/// The labelled stories (committed ones always; optional ones when present).
fn stories() -> Vec<Story> {
    let labels: Value = serde_json::from_str(&read("golden/fixtures/music_mood/labels.json"))
        .expect("labels.json parses");
    let extra_root = std::env::var("MOTION_MOOD_EXTRA_ROOT")
        .ok()
        .map(PathBuf::from);
    let mut out = Vec::new();
    for (key, optional) in [("stories", false), ("optional", true)] {
        let Some(list) = labels[key].as_array() else {
            panic!("labels.json has no `{key}` list");
        };
        for e in list {
            let path = e["path"].as_str().expect("path").to_string();
            let load = |rel: &str| -> Option<String> {
                let p = repo().join(rel);
                if p.exists() {
                    return std::fs::read_to_string(p).ok();
                }
                if optional {
                    let root = extra_root.as_ref()?;
                    return std::fs::read_to_string(root.join(rel)).ok();
                }
                panic!("committed labelled file is missing: {rel}");
            };
            let Some(text) = load(&path) else {
                continue; // optional and absent
            };
            let intent = CreativeIntent::from_json(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
            let (own_style, has_style) = match e["style"].as_str().and_then(&load) {
                Some(s) => (
                    StyleProfile::from_json(&s).unwrap_or_else(|e| panic!("{path} style: {e}")),
                    true,
                ),
                None => (StyleProfile::default(), false),
            };
            out.push(Story {
                name: stem(&path),
                expected: family(e["expected"].as_str().expect("expected")),
                accepted: e["accepted"]
                    .as_array()
                    .expect("accepted")
                    .iter()
                    .map(|v| family(v.as_str().expect("family")))
                    .collect(),
                own_style,
                has_style,
                intent,
                optional,
            });
        }
    }
    out
}

fn committed(all: &[Story]) -> Vec<&Story> {
    all.iter().filter(|s| !s.optional).collect()
}

fn tone_style(tone: &str) -> StyleProfile {
    StyleProfile::from_json(&format!(r#"{{"tone":"{tone}"}}"#)).expect("tone style")
}

fn mood_of(intent: &CreativeIntent, style: &StyleProfile) -> MoodProfile {
    story_mood(intent, &resolve(style))
}

fn mood_for(story: &Story, style: &StyleProfile) -> MoodProfile {
    mood_of(&story.intent, style)
}

fn seed_for(story: &Story, take: u64) -> u64 {
    let statements: Vec<&str> = story
        .intent
        .beats
        .iter()
        .map(|b| b.statement.as_str())
        .collect();
    take_seed(story_key(&story.intent.title, &statements), take)
}

/// Mood and bed under `tone`, take `take`.
fn bed_for(cat: &MusicCatalog, story: &Story, tone: &str, take: u64) -> (MoodProfile, BedChoice) {
    let mood = mood_for(story, &tone_style(tone));
    let choice = select_bed(cat, &mood, MusicWord::Auto, seed_for(story, take));
    (mood, choice)
}

fn find<'a>(all: &'a [Story], name: &str) -> &'a Story {
    all.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no labelled story {name}"))
}

fn bed_id(c: &BedChoice) -> &str {
    c.bed.as_deref().unwrap_or("(none)")
}

fn names(list: &[MoodFamily]) -> String {
    list.iter()
        .map(|f| f.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Catalog v0.2
// ---------------------------------------------------------------------------

#[test]
fn catalog_v02_carries_the_coordinator_tags_and_measurements() {
    let cat = catalog();
    assert_eq!(cat.version, "0.2");
    // (id, moods, energy, tempo)
    let table: [(&str, &[&str], u8, TempoBand); 7] = [
        ("warm_piano", &["calm", "inspiring"], 2, TempoBand::Slow),
        (
            "calm_ambient",
            &["calm", "wonder", "somber"],
            1,
            TempoBand::Slow,
        ),
        (
            "bright_upbeat",
            &["upbeat", "playful"],
            4,
            TempoBand::Medium,
        ),
        (
            "tech_pulse",
            &["neutral_explainer", "serious"],
            3,
            TempoBand::Medium,
        ),
        (
            "cinematic_strings",
            &["dramatic", "wonder"],
            4,
            TempoBand::Medium,
        ),
        ("driving_pulse", &["tense", "upbeat"], 5, TempoBand::Fast),
        ("retro_groove", &["playful", "upbeat"], 3, TempoBand::Medium),
    ];
    assert_eq!(cat.beds.len(), table.len());
    for (id, moods, energy, tempo) in table {
        let bed = cat
            .beds
            .iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("bed {id}"));
        let want: Vec<MoodFamily> = moods.iter().map(|m| family(m)).collect();
        assert_eq!(bed.moods, want, "{id} moods");
        assert_eq!(bed.energy, energy, "{id} energy");
        assert_eq!(bed.tempo_band, Some(tempo), "{id} tempo band");
        assert!(!bed.emotions.is_empty(), "{id} keeps its v0.1 emotions");
        // lra comes from the bed's MusicPlan.
        let plan: Value = serde_json::from_str(&read(&format!("assets/music/{}", bed.plan)))
            .unwrap_or_else(|e| panic!("{id} plan: {e}"));
        assert_eq!(bed.lra, plan["lra"].as_f64(), "{id} lra is the plan's");
        let sb = bed
            .speech_band_db
            .unwrap_or_else(|| panic!("{id} speech_band_db"));
        assert!(
            (-20.0..0.0).contains(&sb),
            "{id} speech_band_db {sb} is a level below full band"
        );
    }
    // The v0.1 consumers read the file as plain JSON: ids, plans, emotions, bpm.
    let raw: Value = serde_json::from_str(&read("assets/music/catalog.json")).expect("json");
    for b in raw["beds"].as_array().expect("beds") {
        assert!(b["id"].is_string() && b["plan"].is_string() && b["bpm"].is_number());
        assert!(b["emotions"].as_array().is_some_and(|e| !e.is_empty()));
    }
}

#[test]
fn compatibility_table_is_the_coordinators() {
    use MoodFamily::*;
    let table: [(MoodFamily, &[MoodFamily]); 10] = [
        (NeutralExplainer, &[NeutralExplainer, Serious, Calm]),
        (Serious, &[Serious, NeutralExplainer, Somber]),
        (Somber, &[Somber, Serious, Calm]),
        (Wonder, &[Wonder, Calm, Inspiring, Dramatic]),
        (Upbeat, &[Upbeat, Playful, Inspiring]),
        (Playful, &[Playful, Upbeat]),
        (Dramatic, &[Dramatic, Tense, Wonder]),
        (Tense, &[Tense, Dramatic, Serious]),
        (Calm, &[Calm, Inspiring, NeutralExplainer]),
        (Inspiring, &[Inspiring, Upbeat, Calm]),
    ];
    for (f, want) in table {
        assert_eq!(compatible_families(f), want, "{f:?}");
    }
}

// ---------------------------------------------------------------------------
// Criterion 1: the family read matches the labels
// ---------------------------------------------------------------------------

#[test]
fn labelled_stories_read_the_accepted_family() {
    let all = stories();
    let base = committed(&all);
    assert!(
        base.len() >= 34,
        "at least 34 committed labelled stories (got {})",
        base.len()
    );
    let mut misses = Vec::new();
    let mut exact = 0usize;
    for s in &base {
        let mood = mood_for(s, &s.own_style);
        if mood.family == s.expected {
            exact += 1;
        }
        if !s.accepted.contains(&mood.family) {
            misses.push(format!(
                "{} read {} (expected {}, accepted {})",
                s.name,
                mood.family.as_str(),
                s.expected.as_str(),
                names(&s.accepted)
            ));
        }
    }
    let ok = base.len() - misses.len();
    let share = ok as f64 / base.len() as f64;
    println!(
        "labelled stories: {} committed; read in the accepted set {}/{} = {:.1} %; equal to the expected family {}/{}",
        base.len(),
        ok,
        base.len(),
        share * 100.0,
        exact,
        base.len()
    );
    for m in &misses {
        println!("  MISS {m}");
    }
    if all.len() > base.len() {
        let extra_ok = all
            .iter()
            .filter(|s| s.accepted.contains(&mood_for(s, &s.own_style).family))
            .count();
        println!(
            "with the {} optional stories present: {}/{} = {:.1} %",
            all.len() - base.len(),
            extra_ok,
            all.len(),
            100.0 * extra_ok as f64 / all.len() as f64
        );
    }
    assert!(
        share >= SHARE_FLOOR,
        "{:.1} % < {:.0} %; misses: {misses:?}",
        share * 100.0,
        SHARE_FLOOR * 100.0
    );
}

/// A probe story: (title, accepted families, [(statement, narration)]).
type Probe = (
    &'static str,
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);

/// Batch A: written after the lexicon was first settled. First run: 9 of 12 in
/// the accepted set (layoff_notice, marathon_training and ancient_library
/// read as a neutral explainer); the lexicon was then extended with the
/// vocabulary those three showed missing (layoffs and jobs, training, ancient
/// and scrolls), so batch A is a regression set now.
const PROBES_A: [Probe; 12] = [
    (
        "coral_reefs",
        &["wonder", "calm", "inspiring"],
        &[
            ("Reefs are crowded cities", "Coral reefs cover less than one percent of the seafloor, yet a quarter of all marine life lives there."),
            ("Thousands of species", "A single reef can be home to thousands of different species."),
            ("Keep them alive", "Protect them, and the ocean keeps its richest neighbourhoods."),
        ],
    ),
    (
        "birthday_surprise",
        &["upbeat", "playful", "inspiring"],
        &[
            ("Everyone hid in the kitchen", "Seven friends hid in the kitchen with balloons and a huge cake."),
            ("Surprise!", "When she opened the door, everyone yelled surprise and the music started."),
            ("Best birthday ever", "She laughed so hard she cried, and the party went on until midnight."),
        ],
    ),
    (
        "layoff_notice",
        &["somber", "serious"],
        &[
            ("A Tuesday morning", "On a Tuesday morning, forty people were told that their jobs were gone."),
            ("Twenty years of work", "Some of them had worked there for twenty years."),
            ("Still looking", "Many of them are still looking for work, and their families are struggling."),
        ],
    ),
    (
        "marathon_training",
        &["inspiring", "calm", "upbeat"],
        &[
            ("One mile first", "Nobody runs a marathon in a day. You begin with one mile."),
            ("A little more each week", "Add a little each week, and your body adapts."),
            ("The finish line", "By spring, the finish line is yours."),
        ],
    ),
    (
        "meeting_rules",
        &["neutral_explainer", "serious", "calm"],
        &[
            ("Most meetings are emails", "Most meetings could have been an email. Here is how to tell which ones."),
            ("Decide or send", "If nobody needs to decide anything, send a message instead."),
            ("Fifteen minutes", "Keep the rest to fifteen minutes, with a written summary."),
        ],
    ),
    (
        "bread_baking",
        &["calm", "inspiring", "neutral_explainer"],
        &[
            ("Good bread takes time", "Good bread takes time. Mix flour, water and salt, then let it rest overnight."),
            ("The slow morning", "In the morning, the dough has doubled, soft and slow."),
            ("Bake it", "Bake it until the crust sings."),
        ],
    ),
    (
        "tournament_final",
        &["tense", "dramatic"],
        &[
            ("One last corner", "Ninety minutes gone, the score level, and one last corner kick to play."),
            ("The stadium holds its breath", "The goalkeeper walks up for it, and the whole stadium holds its breath."),
            ("Now or never", "The ball swings in. It is now or never."),
        ],
    ),
    (
        "wifi_router",
        &["neutral_explainer", "calm", "upbeat"],
        &[
            ("Why it is slow", "Your wifi is slow because the router sits behind the sofa."),
            ("Move it", "Move it to the middle of the room and lift it off the floor."),
            ("Twice the signal", "Signal strength can double with those two small changes."),
        ],
    ),
    (
        "ancient_library",
        &["serious", "wonder", "somber"],
        &[
            ("The great library", "The great library of Alexandria held hundreds of thousands of scrolls."),
            ("A meeting of minds", "Scholars from across the ancient world came there to study."),
            ("What was lost", "What was lost still fascinates us today."),
        ],
    ),
    (
        "cat_antics",
        &["playful", "upbeat", "wonder", "neutral_explainer"],
        &[
            ("Why do cats push cups?", "Why do cats knock things off tables? Honestly, it is a little funny."),
            ("The stare", "They stare right at you and push the cup anyway."),
            ("No good answer", "Science still has no good answer."),
        ],
    ),
    (
        "typhoon_warning",
        &["tense", "dramatic", "somber", "serious"],
        &[
            ("It is heading for the coast", "A typhoon is moving toward the coast, and the warning has just been raised."),
            ("Leaving home", "Thousands of people are leaving their homes before the storm arrives."),
            ("Tonight", "By tonight, the wind will be at its worst."),
        ],
    ),
    (
        "quiet_minutes",
        &["calm", "inspiring"],
        &[
            ("Five quiet minutes", "Five minutes of quiet can change how your whole day feels."),
            ("Breathe", "Close your eyes. Breathe in slowly, and out again."),
            ("Every morning", "Do it every morning and notice the difference."),
        ],
    ),
];

/// Batch B: written after batch A had tuned the lexicon, and run once before
/// anything was changed for it (the generalisation estimate in the report).
const PROBES_B: [Probe; 14] = [
    (
        "electric_cars",
        &["neutral_explainer", "serious", "upbeat", "inspiring"],
        &[
            (
                "Fewer moving parts",
                "Electric cars have far fewer moving parts than petrol cars.",
            ),
            (
                "Fewer repairs",
                "That means fewer repairs and lower running costs over time.",
            ),
            (
                "Charge at home",
                "Charging at home is usually the cheapest way to fill up.",
            ),
        ],
    ),
    (
        "pandemic_lessons",
        &["somber", "serious"],
        &[
            (
                "A new virus",
                "In 2020, a new virus spread across the world in a matter of weeks.",
            ),
            (
                "Hospitals full",
                "Hospitals filled up, and millions of families lost someone they loved.",
            ),
            (
                "Health, for good",
                "The pandemic changed how we think about health forever.",
            ),
        ],
    ),
    (
        "space_telescope",
        &["wonder", "inspiring", "calm", "dramatic"],
        &[
            (
                "Looking back",
                "The James Webb telescope looks back more than thirteen billion years.",
            ),
            (
                "Young galaxies",
                "It sees galaxies as they were when the universe was young.",
            ),
            (
                "Our beginnings",
                "Every image is a window into our own beginnings.",
            ),
        ],
    ),
    (
        "morning_routine",
        &["inspiring", "calm", "upbeat", "neutral_explainer"],
        &[
            (
                "Start the day well",
                "Great days usually start with a good morning.",
            ),
            (
                "Three things",
                "Wake up, drink water, and write down your three goals.",
            ),
            ("Small habits", "Small habits compound into big changes."),
        ],
    ),
    (
        "phone_alerts",
        &["serious", "neutral_explainer", "calm", "inspiring"],
        &[
            (
                "A hundred checks",
                "The average person checks their phone more than a hundred times a day.",
            ),
            (
                "Each ping costs",
                "Each notification pulls your attention away from what matters.",
            ),
            ("Take it back", "Turn off alerts and take back your focus."),
        ],
    ),
    (
        "robot_dance",
        &["playful", "upbeat"],
        &[
            (
                "It can dance",
                "This robot learned to dance, and honestly, it is better than me.",
            ),
            (
                "Never misses a beat",
                "It spins, it jumps, and it never misses a beat.",
            ),
            ("Watch out", "Look out, human dancers."),
        ],
    ),
    (
        "bank_heist",
        &["tense", "dramatic"],
        &[
            (
                "Three minutes",
                "Three minutes before the guards return, the safe is still locked.",
            ),
            (
                "One wrong move",
                "The clock is ticking, and one wrong move sets off the alarm.",
            ),
            ("Go", "Cut the wire, grab the bag, and run."),
        ],
    ),
    (
        "rainy_day",
        &["calm", "inspiring", "neutral_explainer"],
        &[
            (
                "Rain on the window",
                "Rain taps softly on the window while the kettle warms.",
            ),
            (
                "Nowhere to be",
                "There is nowhere to be, and nothing to do.",
            ),
            ("Enough", "A good book and a warm blanket are enough."),
        ],
    ),
    (
        "tax_season",
        &["neutral_explainer", "serious", "calm"],
        &[
            (
                "Every spring",
                "Every spring, millions of people file their tax returns.",
            ),
            (
                "Three weeks",
                "Most refunds arrive within three weeks if you file online.",
            ),
            (
                "Keep receipts",
                "Keep every receipt, and check your numbers twice.",
            ),
        ],
    ),
    (
        "forest_loss",
        &["somber", "serious", "wonder"],
        &[
            (
                "Every minute",
                "Every minute, a forest the size of several football pitches disappears.",
            ),
            (
                "Homes lost",
                "Animals lose their homes, and the climate warms faster.",
            ),
            ("What remains", "What we plant today decides what remains."),
        ],
    ),
    (
        "startup_day",
        &["inspiring", "upbeat", "dramatic", "neutral_explainer"],
        &[
            (
                "Two years later",
                "After two years of work, our small team is finally launching.",
            ),
            (
                "A garage",
                "We built it in a garage with more coffee than money.",
            ),
            ("Thank you", "Thank you to everyone who believed in us."),
        ],
    ),
    (
        "deep_sea",
        &["wonder", "calm", "dramatic"],
        &[
            (
                "Below the light",
                "Below two hundred metres, sunlight fades away completely.",
            ),
            (
                "Glowing animals",
                "Strange animals glow in the dark to find food and each other.",
            ),
            (
                "Less than the moon",
                "We have explored less of the deep sea than the surface of the moon.",
            ),
        ],
    ),
    (
        "hill_castle",
        &["serious", "wonder", "calm", "somber"],
        &[
            (
                "Built to guard",
                "In the twelfth century, the castle was built to guard the valley.",
            ),
            (
                "Never taken",
                "For three hundred years, no army managed to take it.",
            ),
            (
                "A quiet ruin",
                "Today its walls are a quiet ruin on a hill.",
            ),
        ],
    ),
    (
        "pineapple_pizza",
        &["playful", "upbeat", "neutral_explainer"],
        &[
            (
                "A crime?",
                "Is pineapple on pizza a crime? The internet cannot decide.",
            ),
            (
                "Love or fury",
                "Some people love it, some people are furious.",
            ),
            (
                "A silly argument",
                "Honestly, it is just a silly argument about toppings.",
            ),
        ],
    ),
];

/// Batch C (fix round 1): the reviewer's 16 fresh stories, rebuilt from their
/// titles and the time-of-day / rest wording they quoted (the reviewer's own
/// text was not available, so the wording here is mine). Written and run
/// BEFORE any vocabulary was added for them, after the time-of-day words were
/// removed and the grief / hardship company rules went in; the first-run share
/// is in the fix-round report.
const PROBES_C: [Probe; 23] = [
    (
        "rent_vs_buy",
        &["neutral_explainer", "serious"],
        &[
            ("Rent or buy?", "Renting costs less every month, but buying builds equity over time."),
            ("The hidden costs", "A mortgage ties up your savings for thirty years, and repairs add up."),
            ("Run the numbers", "Compare the total cost over ten years before you decide."),
        ],
    ),
    (
        "earthquake_aftermath",
        &["somber", "serious"],
        &[
            ("Dawn over the ruins", "At dawn, rescue teams were still digging through the rubble of the earthquake."),
            ("Nowhere to sleep", "Thousands of families have nowhere to sleep, and the temperatures are falling."),
            ("Counting the dead", "By morning, the number of people killed had passed four hundred."),
        ],
    ),
    (
        "moons_of_jupiter",
        &["wonder", "calm", "inspiring", "dramatic"],
        &[
            ("A crowd of moons", "Jupiter has more than ninety moons, and new ones are still being found."),
            ("An ocean under ice", "One of them, Europa, hides an ocean beneath its icy shell."),
            ("Is anyone there?", "Scientists wonder whether something might be living in that dark water."),
        ],
    ),
    (
        "octopus_fun_fact",
        &["playful", "upbeat", "wonder"],
        &[
            ("Fun fact", "Here is a fun fact: an octopus has three hearts."),
            ("Two for the gills", "Two pump blood to the gills, and one pumps it around the body."),
            ("Weird, right?", "Weird, right? And its blood is blue."),
        ],
    ),
    (
        "two_minute_habit",
        &["inspiring", "calm", "upbeat"],
        &[
            ("Start tiny", "Start with a habit so small that it feels silly."),
            ("Two minutes", "Two minutes a day beats a whole hour once a week."),
            ("It adds up", "Small steps add up until they become who you are."),
        ],
    ),
    (
        "printing_press",
        &["serious", "somber", "neutral_explainer"],
        &[
            ("A press in Mainz", "In 1450, Johannes Gutenberg built a press that could print a page in minutes."),
            ("Books everywhere", "Within fifty years, millions of books were in circulation across Europe."),
            ("Knowledge unlocked", "Knowledge was no longer locked away in monasteries."),
        ],
    ),
    (
        "tax_deadline_tonight",
        &["tense", "dramatic", "serious"],
        &[
            ("Deadline tonight", "The tax deadline is tonight, and millions of people are still filing."),
            ("The site is slowing", "The website is slowing down, and every minute counts."),
            ("A few hours left", "You have a few hours left, so start now."),
        ],
    ),
    (
        "evening_wind_down",
        &["calm", "neutral_explainer", "inspiring"],
        &[
            ("Wind down", "As the evening settles in, it is time to slow down."),
            ("Lights low", "Dim the lights, put the phone away, and breathe."),
            ("A quiet hour", "A quiet hour before bed is the best gift you can give your sleep."),
        ],
    ),
    (
        "hurricane_aftermath",
        &["somber", "serious"],
        &[
            ("The storm came at night", "The hurricane hit the coast at night, and the sea poured over the harbour wall."),
            ("The town in the morning", "In the morning, the survivors found flooded streets and roofs torn off their homes."),
            ("Hundreds missing", "Hundreds of people are still missing, and thousands are sleeping in school halls."),
        ],
    ),
    (
        "hospital_night_shift",
        &["somber", "serious"],
        &[
            ("Seven to seven", "On the night shift, the nurses work twelve hours without rest."),
            ("Every bed is full", "The ward is full, and new patients keep arriving all the way to morning."),
            ("Running on empty", "Many of the staff are exhausted, and some patients will not make it through the night."),
        ],
    ),
    (
        "earthquake_no_time_words",
        &["somber", "serious"],
        &[
            ("The ground gave way", "The earthquake flattened whole streets in less than a minute."),
            ("Digging for survivors", "Rescue teams are digging through the rubble, looking for survivors."),
            ("Left with nothing", "Thousands of families have lost their homes and everything in them."),
        ],
    ),
    (
        "chernobyl",
        &["serious", "somber", "neutral_explainer"],
        &[
            ("April 1986", "In April 1986, reactor four at Chernobyl exploded, and the worst nuclear disaster in history began."),
            ("The cloud", "A cloud of radiation drifted across Europe, and tens of thousands of people were evacuated."),
            ("Pripyat today", "The city of Pripyat is still empty today, and the exclusion zone will last for centuries."),
        ],
    ),
    (
        "morning_stretch",
        &["calm", "neutral_explainer", "inspiring"],
        &[
            ("Two minutes", "Start the morning with a two minute stretch."),
            ("Reach and bend", "Reach up, bend forward, and breathe slowly."),
            ("Thank your back", "Your back will thank you all day."),
        ],
    ),
    (
        "salary_negotiation",
        &["neutral_explainer", "serious", "inspiring"],
        &[
            ("Ask for more", "Most people never negotiate their first salary."),
            ("Know the market", "Research the market rate, then ask for ten percent more."),
            ("Worst case", "The worst they can say is no, and you keep the offer."),
        ],
    ),
    (
        "black_friday_prices",
        &["neutral_explainer", "serious", "upbeat"],
        &[
            ("Is the deal real?", "Many Black Friday discounts are not real."),
            ("The October trick", "Some stores raise their prices in October, then cut them back in November."),
            ("Track first", "Track the price for a month before you buy."),
        ],
    ),
    (
        "viking_raids",
        &["serious", "dramatic", "wonder"],
        &[
            ("793", "In the year 793, Viking raiders attacked a monastery on the English coast."),
            ("Three centuries", "For three centuries, their longships terrorised the coasts of Europe."),
            ("From raiders to settlers", "Then the raiders became traders and settlers."),
        ],
    ),
    // --- fix round 2: the reviewer's second set (their wording; three beats
    // each, one sentence per beat where they gave three sentences).
    (
        "hospice_dying",
        &["somber", "serious"],
        &[
            ("Care at the end", "In a hospice, nurses help people who are dying spend their last days in comfort."),
            ("Calm and gentle", "The rooms are calm, the voices are gentle, and nothing is rushed."),
            ("Time to sleep", "Patients sleep through most of their last days, and their families sit beside them."),
        ],
    ),
    (
        "old_dog_dies",
        &["somber", "serious"],
        &[
            ("One day", "Every dog grows old, and one day it dies."),
            ("Rest in peace", "We say rest in peace, and we mean calm and gentle."),
            ("Naps in the sun", "Until then, the old dog naps in the sun and wants nothing more."),
        ],
    ),
    (
        "sleep_tip_afternoon",
        &["calm", "neutral_explainer", "inspiring"],
        &[
            ("The afternoon slump", "Always tired in the afternoon? You may simply need to sleep a little more."),
            ("Earlier to bed", "Go to bed half an hour earlier, and keep the lights low."),
            ("Calm evenings", "Calm evenings make for better nights, and better afternoons."),
        ],
    ),
    (
        "sleep_tip_protects",
        &["calm", "neutral_explainer", "inspiring"],
        &[
            ("Sleep protects you", "Good sleep protects you from illness."),
            ("A tired brain", "An exhausted brain cannot focus."),
            ("Rest well", "Rest well, and relax before bed."),
        ],
    ),
    (
        "hurricane_island",
        &["somber", "serious"],
        &[
            ("The hurricane hit at night", "The hurricane hit the island at night."),
            ("Half the homes had no roof", "In the morning, half the homes had no roof."),
            ("Waiting for clean water", "A week later, families are still waiting for clean water."),
        ],
    ),
    (
        "chernobyl_reactor",
        &["somber", "serious"],
        &[
            ("A reactor exploded", "In April 1986, a reactor at Chernobyl exploded."),
            ("The cloud", "Radiation drifted across Europe, and the nearby town was evacuated within two days."),
            ("Still empty", "The town is still empty, and the exclusion zone will last for centuries."),
        ],
    ),
    (
        // The reviewer's exact text (C4a review round 2): "exploded" is its only
        // spectacle word, so a human-cost cue must keep it off the strings.
        "chernobyl_reviewer",
        &["somber", "serious", "neutral_explainer"],
        &[
            ("Reactor four", "In April 1986, reactor four at Chernobyl exploded."),
            ("Pripyat", "Fifty thousand people left Pripyat and never came back."),
            ("Still closed", "The zone around it is still closed today."),
        ],
    ),
];

/// The disaster, death and hardship stories of batch C: under no tone may they
/// get a calm, inspiring, upbeat, playful, dramatic or wonder bed (the somber
/// bed `calm_ambient` and the serious bed `tech_pulse` are the only beds that
/// fit); in particular never `cinematic_strings` or `warm_piano`.
const HARDSHIP_PROBES: [&str; 10] = [
    "earthquake_aftermath",
    "hurricane_aftermath",
    "hospital_night_shift",
    "earthquake_no_time_words",
    "chernobyl",
    "hospice_dying",
    "old_dog_dies",
    "hurricane_island",
    "chernobyl_reactor",
    "chernobyl_reviewer",
];

fn held_out_intent(title: &str, lines: &[(&str, &str)]) -> CreativeIntent {
    let beats: Vec<Value> = lines
        .iter()
        .map(|(statement, narration)| {
            serde_json::json!({
                "purpose": "emphasize",
                "statement": statement,
                "narration": narration,
                "primary": { "kind": "phrase", "meaning": "the story" },
            })
        })
        .collect();
    let doc = serde_json::json!({ "version": "0.2", "title": title, "beats": beats });
    CreativeIntent::from_value(doc).expect("held-out intent")
}

fn probe_run(label: &str, probes: &[Probe]) -> (usize, Vec<String>) {
    let mut misses = Vec::new();
    for (title, accepted, lines) in probes {
        let intent = held_out_intent(title, lines);
        let mood = mood_of(&intent, &StyleProfile::default());
        let accepted: Vec<MoodFamily> = accepted.iter().map(|f| family(f)).collect();
        println!(
            "{label} {title:<18} read {:<18} ({})",
            mood.family.as_str(),
            mood.reason
        );
        if !accepted.contains(&mood.family) {
            misses.push(format!("{title}: read {}", mood.family.as_str()));
        }
    }
    (probes.len() - misses.len(), misses)
}

#[test]
fn probe_stories_outside_the_labelled_set() {
    for (label, probes) in [("batch A", &PROBES_A[..]), ("batch B", &PROBES_B[..])] {
        let (ok, misses) = probe_run(label, probes);
        println!(
            "{label}: {ok}/{} in the accepted set; misses {misses:?}",
            probes.len()
        );
        assert!(
            ok as f64 / probes.len() as f64 >= SHARE_FLOOR,
            "{label} misses: {misses:?}"
        );
    }
}

#[test]
fn probe_batch_c_and_the_hardship_stories() {
    let (ok, misses) = probe_run("batch C", &PROBES_C);
    println!(
        "batch C: {ok}/{} in the accepted set; misses {misses:?}",
        PROBES_C.len()
    );

    // Hard rule: a disaster or hardship story never gets a calm, inspiring,
    // upbeat or playful bed, under any of the 9 tones, and always reads somber
    // or serious.
    let cat = catalog();
    for name in HARDSHIP_PROBES {
        let (_, _, lines) = PROBES_C
            .iter()
            .find(|(t, _, _)| *t == name)
            .unwrap_or_else(|| panic!("probe {name}"));
        let intent = held_out_intent(name, lines);
        for tone in TONES {
            let mood = mood_of(&intent, &tone_style(tone));
            assert!(
                matches!(mood.family, MoodFamily::Somber | MoodFamily::Serious),
                "{name} under {tone} reads {} ({})",
                mood.family.as_str(),
                mood.reason
            );
            for seed in 0..4u64 {
                let c = select_bed(&cat, &mood, MusicWord::Auto, seed);
                assert!(
                    matches!(
                        c.bed.as_deref(),
                        None | Some("calm_ambient") | Some("tech_pulse")
                    ),
                    "{name} under {tone}: {:?}",
                    c.bed
                );
            }
        }
    }
    assert!(
        ok as f64 / PROBES_C.len() as f64 >= SHARE_FLOOR,
        "batch C misses: {misses:?}"
    );
}

#[test]
fn a_spectacle_story_without_hardship_keeps_its_strings() {
    // The company rules must not touch a story of force with no grief or
    // hardship cue: the volcano ("the fire dies down", no rest words) stays
    // dramatic, space and the ocean stories stay wonder with strings.
    let cat = catalog();
    let all = stories();
    let volcano = find(&all, "volcano_eruption");
    for tone in TONES {
        let (mood, c) = bed_for(&cat, volcano, tone, 0);
        assert_eq!(mood.family, MoodFamily::Dramatic, "volcano under {tone}");
        assert_eq!(c.bed.as_deref(), Some("cinematic_strings"), "{tone}");
    }
    let space = find(&all, "space");
    assert_eq!(
        bed_for(&cat, space, "auto", 0).1.bed.as_deref(),
        Some("cinematic_strings")
    );
}

// ---------------------------------------------------------------------------
// Criterion 2: hard rules
// ---------------------------------------------------------------------------

#[test]
fn hard_rules_hold_for_every_story_tone_and_take() {
    let cat = catalog();
    let all = stories();
    let mut checked = 0usize;
    for s in &all {
        for tone in TONES {
            let mood = mood_for(s, &tone_style(tone));
            assert_eq!(
                mood.compatible,
                compatible_families(mood.family),
                "{} / {tone}",
                s.name
            );
            assert_eq!(mood.compatible[0], mood.family);
            assert!(
                (1..=5).contains(&mood.energy),
                "{} / {tone}: energy",
                s.name
            );
            for take in TAKES {
                let choice = select_bed(&cat, &mood, MusicWord::Auto, seed_for(s, take));
                let ctx = format!("{} / {tone} / take {take}", s.name);
                assert_eq!(choice.mood, mood.family, "{ctx}");
                assert!(choice.warning.is_none(), "{ctx}: auto never warns");
                if let Some(id) = &choice.bed {
                    let bed = cat
                        .beds
                        .iter()
                        .find(|b| &b.id == id)
                        .expect("bed in catalog");
                    if id == "cinematic_strings" {
                        assert!(
                            matches!(mood.family, MoodFamily::Dramatic | MoodFamily::Wonder),
                            "{ctx}: strings on a {} story",
                            mood.family.as_str()
                        );
                    }
                    assert!(
                        bed.moods.iter().any(|m| mood.compatible.contains(m)),
                        "{ctx}: {id} moods {:?} miss {:?}",
                        bed.moods,
                        mood.compatible
                    );
                    assert!(
                        choice.reason.starts_with(id.as_str()),
                        "{ctx}: {}",
                        choice.reason
                    );
                } else {
                    assert!(
                        choice.reason.contains("silence"),
                        "{ctx}: {}",
                        choice.reason
                    );
                }
                checked += 1;
            }
        }
    }
    println!("hard rules checked on {checked} (story, tone, take) cases");
    assert!(checked >= 34 * 9 * 4);
}

#[test]
fn named_story_rules() {
    let cat = catalog();
    let all = stories();
    // compound_interest (tone auto) -> tech_pulse, every take.
    let ci = find(&all, "compound_interest");
    for take in TAKES {
        let (mood, c) = bed_for(&cat, ci, "auto", take);
        assert_eq!(mood.family, MoodFamily::NeutralExplainer);
        assert_eq!(c.bed.as_deref(), Some("tech_pulse"), "take {take}");
        assert_eq!(
            c.reason, "tech_pulse (neutral explainer, money story)",
            "the reply names the bed and why"
        );
    }
    // Documentary: prices and leaks stories get a neutral / serious bed, never warm piano.
    for name in ["weekly_basket", "editorial_demo", "city_water"] {
        let s = find(&all, name);
        for take in TAKES {
            let (mood, c) = bed_for(&cat, s, "documentary", take);
            let id = c.bed.as_deref().unwrap_or_else(|| panic!("{name}: no bed"));
            assert_ne!(id, "warm_piano", "{name} take {take}");
            let bed = cat.beds.iter().find(|b| b.id == id).expect("bed");
            assert!(
                bed.moods
                    .iter()
                    .any(|m| matches!(m, MoodFamily::NeutralExplainer | MoodFamily::Serious)),
                "{name}: {id} is not a neutral / serious bed ({})",
                mood.family.as_str()
            );
        }
    }
    // space (cinematic): ambient or strings.
    let space = find(&all, "space");
    for style in [space.own_style.clone(), tone_style("cinematic")] {
        let mood = mood_for(space, &style);
        for take in TAKES {
            let c = select_bed(&cat, &mood, MusicWord::Auto, seed_for(space, take));
            assert!(
                matches!(
                    c.bed.as_deref(),
                    Some("calm_ambient") | Some("cinematic_strings")
                ),
                "space take {take}: {:?}",
                c.bed
            );
        }
    }
    // flood_toll: calm_ambient or no bed, under every tone and take.
    let flood = find(&all, "flood_toll");
    for tone in TONES {
        for take in TAKES {
            let (_, c) = bed_for(&cat, flood, tone, take);
            assert!(
                matches!(c.bed.as_deref(), None | Some("calm_ambient")),
                "flood_toll {tone} take {take}: {:?}",
                c.bed
            );
        }
    }
}

#[test]
fn the_tone_reaches_selection_through_energy() {
    let all = stories();
    let s = find(&all, "sleep_review");
    let energy = |tone: &str| mood_for(s, &tone_style(tone)).energy;
    assert_eq!(energy("hype"), 5);
    assert_eq!(energy("playful"), 5);
    assert_eq!(energy("street"), 5);
    assert_eq!(energy("documentary"), 1);
    assert_eq!(energy("editorial"), 1);
    // The other tones read the beats: sleep_review is impact, building x3, calm.
    for tone in ["auto", "technical", "studio", "cinematic"] {
        assert_eq!(energy(tone), 3, "{tone}");
    }
    // The family does not move with the energy.
    let fam = |tone: &str| mood_for(s, &tone_style(tone)).family;
    assert_eq!(fam("hype"), fam("documentary"));
}

#[test]
fn a_wonder_story_takes_the_quiet_or_the_big_bed_by_tone() {
    let cat = catalog();
    let all = stories();
    let space = find(&all, "space");
    let bed = |tone: &str| bed_for(&cat, space, tone, 0).1.bed;
    assert_eq!(bed("documentary").as_deref(), Some("calm_ambient"));
    assert_eq!(bed("hype").as_deref(), Some("cinematic_strings"));
    assert_eq!(bed("cinematic").as_deref(), Some("cinematic_strings"));
}

#[test]
fn a_strong_story_is_not_moved_by_the_tone() {
    // The tone is a secondary bias: it never outvotes what the story is about.
    let all = stories();
    for name in [
        "compound_interest",
        "volcano_eruption",
        "flood_toll",
        "space",
    ] {
        let s = find(&all, name);
        let auto = mood_for(s, &tone_style("auto")).family;
        for tone in TONES {
            assert_eq!(
                mood_for(s, &tone_style(tone)).family,
                auto,
                "{name} under {tone}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Criterion 3: the music word, determinism, rotation
// ---------------------------------------------------------------------------

#[test]
fn the_music_word_forces_a_family_and_warns_on_conflict() {
    let cat = catalog();
    let all = stories();
    let ci = find(&all, "compound_interest");
    let mood = mood_for(ci, &tone_style("auto"));
    let seed = seed_for(ci, 0);

    // Forced dramatic on a money story: strings plus a music_fit warning.
    let c = select_bed(&cat, &mood, MusicWord::Dramatic, seed);
    assert_eq!(c.bed.as_deref(), Some("cinematic_strings"));
    assert_eq!(c.mood, MoodFamily::Dramatic);
    let w = c.warning.as_deref().expect("a conflicting word warns");
    assert!(
        w.contains("dramatic") && w.contains("neutral explainer"),
        "{w}"
    );

    // Forced calm on a calm story: no warning.
    let nap = find(&all, "why_we_nap");
    let nap_mood = mood_for(nap, &tone_style("auto"));
    assert_eq!(nap_mood.family, MoodFamily::Calm);
    let c = select_bed(&cat, &nap_mood, MusicWord::Calm, seed_for(nap, 0));
    assert!(c.warning.is_none());
    assert!(matches!(
        c.bed.as_deref(),
        Some("warm_piano") | Some("calm_ambient")
    ));

    // A forced word inside the compatible list does not warn either
    // (serious on a neutral explainer), and neutral means neutral_explainer.
    let c = select_bed(&cat, &mood, MusicWord::Serious, seed);
    assert!(c.warning.is_none());
    assert_eq!(c.bed.as_deref(), Some("tech_pulse"));
    let c = select_bed(&cat, &mood, MusicWord::Neutral, seed);
    assert_eq!(c.mood, MoodFamily::NeutralExplainer);
    assert!(c.warning.is_none());

    // none: no bed, no warning, a reason.
    let c = select_bed(&cat, &mood, MusicWord::None, seed);
    assert!(c.bed.is_none() && c.warning.is_none() && !c.reason.is_empty());

    // Every word on every story is a valid choice: a bed that has the word's
    // family (strings only for dramatic / wonder), never a panic; a warning
    // exactly when the word's family is outside the content's compatible list.
    for s in &all {
        let m = mood_for(s, &tone_style("auto"));
        for word in MusicWord::ALL {
            let c = select_bed(&cat, &m, word, seed_for(s, 0));
            if word == MusicWord::None {
                assert!(c.bed.is_none() && c.warning.is_none());
                continue;
            }
            if let Some(id) = &c.bed {
                let bed = cat.beds.iter().find(|b| &b.id == id).expect("bed");
                assert!(bed.moods.contains(&c.mood), "{} {word:?}: {id}", s.name);
                if id == "cinematic_strings" {
                    assert!(matches!(c.mood, MoodFamily::Dramatic | MoodFamily::Wonder));
                }
            }
            let forced = motion_core::music_mood::forced_family(word);
            let conflict = forced.is_some_and(|f| !m.compatible.contains(&f));
            assert_eq!(c.warning.is_some(), conflict, "{} {word:?}", s.name);
        }
    }
}

#[test]
fn selection_is_deterministic_for_a_seed() {
    let cat = catalog();
    let all = stories();
    for s in &all {
        for tone in TONES {
            let mood = mood_for(s, &tone_style(tone));
            assert_eq!(mood, mood_for(s, &tone_style(tone)), "{} {tone}", s.name);
            for take in TAKES {
                let seed = seed_for(s, take);
                for word in MusicWord::ALL {
                    assert_eq!(
                        select_bed(&cat, &mood, word, seed),
                        select_bed(&cat, &mood, word, seed),
                        "{} {tone} {take} {word:?}",
                        s.name
                    );
                }
            }
        }
    }
}

/// The catalog with one extra bed that tags exactly like `twin` (what a second
/// bed of the same mood and energy would be).
fn catalog_with_twin(twin: &str, new_id: &str) -> MusicCatalog {
    let mut cat = catalog();
    let mut bed = cat
        .beds
        .iter()
        .find(|b| b.id == twin)
        .expect("twin")
        .clone();
    bed.id = new_id.to_string();
    cat.beds.push(bed);
    cat
}

#[test]
fn takes_rotate_only_within_the_best_rank() {
    let all = stories();
    let nap = find(&all, "why_we_nap");
    let mood = mood_for(nap, &tone_style("auto"));
    assert_eq!(mood.family, MoodFamily::Calm);

    // Two beds with the same moods and the same energy are an exact tie: the
    // seed rotates between them, and only between them (calm_ambient, also in
    // rank 1, is farther from the story's energy and never takes a turn).
    let cat = catalog_with_twin("warm_piano", "warm_piano_b");
    let mut seen = BTreeSet::new();
    for take in 0..64u64 {
        let c = select_bed(&cat, &mood, MusicWord::Auto, seed_for(nap, take));
        let id = c.bed.expect("a bed");
        assert!(
            id == "warm_piano" || id == "warm_piano_b",
            "rotation stays inside the tie, got {id}"
        );
        seen.insert(id);
    }
    assert_eq!(
        seen.len(),
        2,
        "both tied beds are used across takes: {seen:?}"
    );
    let shown: Vec<String> = TAKES
        .iter()
        .map(|&t| bed_id(&select_bed(&cat, &mood, MusicWord::Auto, seed_for(nap, t))).to_string())
        .collect();
    println!("why_we_nap, catalog + a twin of warm_piano, takes 0-3: {shown:?}");

    // Under a documentary look (energy 1) calm_ambient is the closest bed, so
    // the twins are not tied with it and never chosen.
    let doc_mood = mood_for(nap, &tone_style("documentary"));
    for take in TAKES {
        let c = select_bed(&cat, &doc_mood, MusicWord::Auto, seed_for(nap, take));
        assert_eq!(c.bed.as_deref(), Some("calm_ambient"), "take {take}");
    }

    // A tie never crosses ranks: a twin of retro_groove (rank 1 of a playful
    // story) rotates with retro_groove only, never with a rank-2 bed.
    let five = find(&all, "retro");
    let pmood = mood_for(five, &tone_style("auto"));
    assert_eq!(pmood.family, MoodFamily::Playful);
    let cat2 = catalog_with_twin("retro_groove", "retro_groove_b");
    let mut seen2 = BTreeSet::new();
    for take in 0..64u64 {
        let c = select_bed(&cat2, &pmood, MusicWord::Auto, seed_for(five, take));
        seen2.insert(c.bed.expect("a bed"));
    }
    let want: BTreeSet<String> = ["retro_groove", "retro_groove_b"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(seen2, want);

    // The real catalog has no two beds with the same moods at the same
    // energy, so the literal rule (closest energy; exact ties rotate) never
    // varies the bed across takes. Measured here and named in the report.
    let real = catalog();
    let mut rotating = 0usize;
    let mut cases = 0usize;
    for s in &all {
        for tone in TONES {
            let mood = mood_for(s, &tone_style(tone));
            let beds: BTreeSet<String> = TAKES
                .iter()
                .map(|&t| {
                    bed_id(&select_bed(&real, &mood, MusicWord::Auto, seed_for(s, t))).to_string()
                })
                .collect();
            cases += 1;
            if beds.len() > 1 {
                rotating += 1;
            }
        }
    }
    println!(
        "real catalog, literal rule: {rotating} of {cases} (story, tone) cases change bed across takes 0-3"
    );

    // Brute force: for every content family and every energy 1..=5, how many
    // seeds-worth of different beds does the literal rule give?
    let mut tied_pairs = 0usize;
    for fam in MoodFamily::ALL {
        for energy in 1..=5u8 {
            let m = MoodProfile {
                family: fam,
                compatible: compatible_families(fam),
                energy,
                reason: "probe".into(),
            };
            let beds: BTreeSet<String> = (0..32u64)
                .map(|seed| bed_id(&select_bed(&real, &m, MusicWord::Auto, seed)).to_string())
                .collect();
            if beds.len() > 1 {
                tied_pairs += 1;
            }
        }
    }
    println!(
        "real catalog, literal rule: {tied_pairs} of {} (family, energy) pairs have an exact energy tie",
        MoodFamily::ALL.len() * 5
    );
}

/// What a one-step tie band would do on the real catalog. The coordinator
/// decides (`ENERGY_TIE_BAND` stays 0); this prints the effect and asserts
/// only that the hard rules still hold and that rotation stays in the best
/// rank.
#[test]
fn what_if_a_one_step_energy_tie_band() {
    let real = catalog();
    let all = stories();
    let mut cases = 0usize;
    let mut rotating = Vec::new();
    for s in &all {
        for tone in TONES {
            let mood = mood_for(s, &tone_style(tone));
            let mut beds = BTreeSet::new();
            for take in TAKES {
                let c =
                    select_bed_with_tie_band(&real, &mood, MusicWord::Auto, seed_for(s, take), 1);
                if let Some(id) = &c.bed {
                    let bed = real.beds.iter().find(|b| &b.id == id).expect("bed");
                    assert!(bed.moods.contains(&mood.family), "rank 1 only, got {id}");
                    if id == "cinematic_strings" {
                        assert!(matches!(
                            mood.family,
                            MoodFamily::Dramatic | MoodFamily::Wonder
                        ));
                    }
                }
                beds.insert(bed_id(&c).to_string());
            }
            cases += 1;
            if beds.len() > 1 {
                rotating.push(format!("{} / {tone}: {beds:?}", s.name));
            }
        }
    }
    println!(
        "what-if band 1: {} of {cases} (story, tone) cases change bed across takes 0-3",
        rotating.len()
    );
    for r in rotating.iter().take(4) {
        println!("  {r}");
    }
}

#[test]
fn no_bed_fits_means_silence_and_rank_two_is_the_second_best() {
    let all = stories();
    let flood = find(&all, "flood_toll");
    let mood = mood_for(flood, &tone_style("auto"));
    assert_eq!(mood.family, MoodFamily::Somber);
    let full = catalog();

    // Only an upbeat bed: its first mood is not in somber's compatible list.
    let mut only_upbeat = full.clone();
    only_upbeat.beds.retain(|b| b.id == "bright_upbeat");
    let c = select_bed(&only_upbeat, &mood, MusicWord::Auto, 7);
    assert!(c.bed.is_none());
    assert_eq!(c.reason, "no bed fits a somber story; silence");

    // No calm_ambient: rank 1 is empty, rank 2 (first mood in somber's list:
    // calm) takes warm_piano.
    let mut no_ambient = full.clone();
    no_ambient.beds.retain(|b| b.id != "calm_ambient");
    let c = select_bed(&no_ambient, &mood, MusicWord::Auto, 7);
    assert_eq!(c.bed.as_deref(), Some("warm_piano"));
    assert!(c.reason.contains("closest fit"), "{}", c.reason);

    // Untagged (v0.1) beds are never eligible.
    let mut untagged = full.clone();
    for b in &mut untagged.beds {
        b.moods.clear();
    }
    let c = select_bed(&untagged, &mood, MusicWord::Auto, 7);
    assert!(c.bed.is_none());

    // Strings never serve a tense story through rank 2.
    let tense = find(&all, "launch_countdown");
    let tmood = mood_for(tense, &tone_style("auto"));
    assert_eq!(tmood.family, MoodFamily::Tense);
    let mut no_driving = full;
    no_driving.beds.retain(|b| b.id != "driving_pulse");
    let c = select_bed(&no_driving, &tmood, MusicWord::Auto, 7);
    assert_ne!(c.bed.as_deref(), Some("cinematic_strings"));
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

#[test]
fn per_story_table() {
    let cat = catalog();
    let all = stories();
    println!();
    println!(
        "{:<24} {:<18} {:<18} {:>2} | {:<18} {:<18} {:<18} {:<18} {:<18}",
        "story",
        "expected",
        "read (own style)",
        "en",
        "auto",
        "documentary",
        "cinematic",
        "hype",
        "playful"
    );
    for s in &all {
        let mood = mood_for(s, &s.own_style);
        let flag = if s.accepted.contains(&mood.family) {
            " "
        } else {
            "!"
        };
        let beds: Vec<String> = ["auto", "documentary", "cinematic", "hype", "playful"]
            .iter()
            .map(|t| bed_id(&bed_for(&cat, s, t, 0).1).to_string())
            .collect();
        println!(
            "{:<24} {:<18} {:<18} {:>2}{flag}| {:<18} {:<18} {:<18} {:<18} {:<18}",
            s.name,
            s.expected.as_str(),
            mood.family.as_str(),
            mood.energy,
            beds[0],
            beds[1],
            beds[2],
            beds[3],
            beds[4]
        );
    }
    println!();
    println!("score margins (own style): winner, runner-up (points), reason");
    for s in &all {
        let scores = score_story(&s.intent, &resolve(&s.own_style));
        let mut ranked = scores.points.clone();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "{:<24} {} {:>3}, {} {:>3}  ({}) style:{}",
            s.name,
            family_label(ranked[0].0),
            ranked[0].1,
            family_label(ranked[1].0),
            ranked[1].1,
            scores.reason,
            if s.has_style { "own" } else { "auto" }
        );
    }
}
