//! (0.23 C4a) Story mood and bed selection: the lexicon, the compatibility
//! table and the scoring behind [`crate::audio::story_mood`] and
//! [`crate::audio::select_bed`].
//!
//! Pure and deterministic: no RNG, no clocks, no iteration over hash maps. The
//! only seed use is `direction::choice(seed, 0, Dim::Bed)`.
//!
//! # How a story's mood is read
//!
//! 1. Every text field a viewer or the narrator meets (narration, statement,
//!    keyword, the `meaning` and written `value` of every subject, collection
//!    items, state changes, metric terms, layer names and notes) is split into
//!    lowercase word tokens. Picture asset names are not read.
//! 2. A lexicon of word patterns (exact word, `prefix*`, or a short phrase)
//!    adds points to a [`MoodFamily`] through a group (money, history,
//!    science, ...). Each pattern counts at most [`TERM_CAP`] times, so one
//!    repeated word cannot decide a story alone.
//! 3. Four readings are group-dependent (company rules): money is a neutral
//!    explainer unless loss words (inflation, debt, leaks, "costs more",
//!    "earning less") are present, then it is serious; history is serious
//!    unless discovery words are present, then it is wonder, and war words are
//!    serious in a history story without grief words, somber otherwise; and
//!    two rules for stories of grief and hardship. (a) Rest words ("sleep",
//!    "rest", "quiet", "calm", "gentle") are mourning, so their points go to
//!    somber, not calm, once the story has any death word (grief points of at
//!    least [`GRIEF_COMPANY`]: dying, died, death, grief ...) or hardship
//!    points of at least [`HARDSHIP_COMPANY`]. (b) Spectacle words ("exploded",
//!    "hurricane", "storm") describe a disaster, so half their points go to
//!    serious, not dramatic, once grief plus hardship points reach
//!    [`HARDSHIP_COMPANY`]. A disaster told only through spectacle words
//!    therefore needs a hardship cue ("no roof", "lost their homes",
//!    "radiation", "evacuated", "aftermath", "rubble"); a spectacle story with
//!    no hardship cue ("the fire dies down") stays dramatic. Death words are
//!    kept apart from idiom-prone words ("takes a toll", "suffer from",
//!    "alone"), which are weak hardship words. Times of day (dawn, morning,
//!    evening, night, midnight) carry no mood and are not in the lexicon;
//!    tiredness ("tired", "exhausted") is a weak rest word, not hardship.
//! 4. Structures lean neutral: a ranking (or a list of three or more figures)
//!    and a run of number-led beats.
//! 5. `neutral_explainer` carries a prior of [`NEUTRAL_PRIOR`] points, so a
//!    single weak word never moves a story and a tie is a neutral explainer.
//! 6. The tone is a secondary bias ([`TONE_FIRST`] / [`TONE_SECOND`] points to
//!    the families the tone leans to). It can only break a near tie, never
//!    outvote a story that says what it is about.
//!
//! Points are integers (no floating point): weak 5, ordinary 10, firm 15,
//! strong 20, decisive 25. No two single-word patterns match the same word (a
//! unit test checks it), so a word counts once through the single-word
//! patterns; a phrase pattern ("wait what", "cost* more") adds its own points
//! on top of the single-word hits its words may also score (the hits are not
//! subtracted).
//!
//! # How the tone reaches bed selection
//!
//! [`crate::audio::select_bed`] has no tone argument (frozen signature), so the
//! tone's energy bias travels in [`MoodProfile::energy`]: a hype or energetic
//! look reads as energy 5 (the highest-energy compatible bed is closest), a
//! documentary or restrained look as energy 1 (the lowest), every other look
//! as the spread of the beats' own energy.
//!
//! # How the catalog's `speech_band_db` was measured
//!
//! `speech_band_method` (kept here because `catalog.json` takes no comments):
//! each bed is down-mixed to mono with ffmpeg `pan=mono|c0=0.5*c0+0.5*c1`, then
//! `volumedetect` reports its `mean_volume` (RMS, dB) over the whole track,
//! once unfiltered and once after two cascaded `highpass=f=300:poles=2` and
//! two cascaded `lowpass=f=4000:poles=2` filters (a 4th-order 300-4000 Hz
//! band). `speech_band_db` is the filtered mean volume minus the unfiltered
//! one; each reading is rounded to 0.1 dB. `lra` is copied from the bed's
//! MusicPlan (`music-index`).

use std::sync::OnceLock;

use crate::audio::{BedChoice, CatalogBed, MoodFamily, MoodProfile, MusicCatalog, MusicWord};
use crate::compiler::direction::{choice, Dim};
use crate::compiler::taste::{Genre, ResolvedStyleProfile, ResolvedTone, TemperamentKind};
use crate::intent::{CollectionItem, CreativeIntent, Energy, Subject};

/// How many times one lexicon pattern may count in a story.
pub const TERM_CAP: usize = 2;
/// Points a neutral explainer starts with: weaker evidence than this is no
/// evidence, and a tie goes to the neutral explainer.
pub const NEUTRAL_PRIOR: i32 = 10;
/// Hardship points from which a story counts as one of hardship (its rest
/// words become mourning), and grief plus hardship points from which its
/// spectacle words describe a disaster (module docs, step 3).
pub const HARDSHIP_COMPANY: i32 = 15;
/// Grief points (one death word) from which a story's rest words become
/// mourning.
pub const GRIEF_COMPANY: i32 = 10;
/// Tone bias points for the first and the second family a tone leans to.
pub const TONE_FIRST: i32 = 6;
pub const TONE_SECOND: i32 = 4;
/// Beds whose distance to the target energy is within this many steps of the
/// closest bed are tied and rotate with the seed. The coordinator's rule is
/// "closest to the energy; remaining ties rotate", i.e. 0 (exact ties only).
pub const ENERGY_TIE_BAND: u8 = 0;
/// The only bed that needs the content to be dramatic or wonder.
const DRAMA_ONLY_BED: &str = "cinematic_strings";

// ---------------------------------------------------------------------------
// Compatibility table (coordinator)
// ---------------------------------------------------------------------------

/// The families a bed may have and still fit content of family `f`, best
/// first (the first entry is `f` itself).
pub fn compatible_families(f: MoodFamily) -> Vec<MoodFamily> {
    use MoodFamily::*;
    match f {
        NeutralExplainer => vec![NeutralExplainer, Serious, Calm],
        Serious => vec![Serious, NeutralExplainer, Somber],
        Somber => vec![Somber, Serious, Calm],
        Wonder => vec![Wonder, Calm, Inspiring, Dramatic],
        Upbeat => vec![Upbeat, Playful, Inspiring],
        Playful => vec![Playful, Upbeat],
        Dramatic => vec![Dramatic, Tense, Wonder],
        Tense => vec![Tense, Dramatic, Serious],
        Calm => vec![Calm, Inspiring, NeutralExplainer],
        Inspiring => vec![Inspiring, Upbeat, Calm],
    }
}

/// "neutral explainer" for `NeutralExplainer`, the family word otherwise.
pub fn family_label(f: MoodFamily) -> String {
    f.as_str().replace('_', " ")
}

/// The family a forced `music` word asks for (`None` for `auto` / `none`).
pub fn forced_family(word: MusicWord) -> Option<MoodFamily> {
    match word {
        MusicWord::Auto | MusicWord::None => None,
        MusicWord::Calm => Some(MoodFamily::Calm),
        MusicWord::Upbeat => Some(MoodFamily::Upbeat),
        MusicWord::Serious => Some(MoodFamily::Serious),
        MusicWord::Dramatic => Some(MoodFamily::Dramatic),
        MusicWord::Playful => Some(MoodFamily::Playful),
        MusicWord::Neutral => Some(MoodFamily::NeutralExplainer),
    }
}

// ---------------------------------------------------------------------------
// Lexicon
// ---------------------------------------------------------------------------

/// What a word is evidence of. Several groups feed one family; some groups
/// change family with their company (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Money,
    Stat,
    Rank,
    Loss,
    Work,
    Civic,
    History,
    Discovery,
    Grief,
    Hardship,
    War,
    Science,
    Space,
    Celebration,
    Boost,
    Trivia,
    Retro,
    Spectacle,
    Deadline,
    Rest,
    Habits,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Group::Money => "money story",
            Group::Stat => "numbers and data",
            Group::Rank => "numbers and rankings",
            Group::Loss => "rising costs and losses",
            Group::Work => "work and office life",
            Group::Civic => "public affairs",
            Group::History => "history",
            Group::Discovery => "discovery",
            Group::Grief => "loss and grief",
            Group::Hardship => "hardship and disaster",
            Group::War => "war",
            Group::Science => "science and nature",
            Group::Space => "space and science",
            Group::Celebration => "celebration",
            Group::Boost => "speed and boosts",
            Group::Trivia => "surprising facts",
            Group::Retro => "retro culture",
            Group::Spectacle => "force and spectacle",
            Group::Deadline => "a ticking clock",
            Group::Rest => "rest and quiet",
            Group::Habits => "habits and goals",
        }
    }
}

/// One lexicon entry: a pattern of space-separated tokens (each an exact word
/// or `prefix*`), the group it is evidence of and its points.
struct Term {
    pat: &'static str,
    group: Group,
    points: i32,
}

const fn t(pat: &'static str, group: Group, points: i32) -> Term {
    Term { pat, group, points }
}

use Group::*;

/// The lexicon. Weak 5, ordinary 10, firm 15, strong 20, decisive 25.
const LEXICON: &[Term] = &[
    // --- money, prices, work and pay: a neutral explainer (serious with loss words)
    t("money", Money, 20),
    t("price*", Money, 20),
    t("salary", Money, 20),
    t("salaries", Money, 20),
    t("wage*", Money, 20),
    t("econom*", Money, 20),
    t("saving*", Money, 20),
    t("save", Money, 10),
    t("saves", Money, 10),
    t("saved", Money, 10),
    t("saver*", Money, 10),
    t("interest", Money, 15),
    t("compound*", Money, 20),
    t("invest", Money, 20),
    t("invests", Money, 20),
    t("invested", Money, 20),
    t("investing", Money, 20),
    t("investment*", Money, 20),
    t("investor*", Money, 20),
    t("budget*", Money, 15),
    t("income", Money, 20),
    t("bank", Money, 10),
    t("banks", Money, 10),
    t("banking", Money, 10),
    t("tax", Money, 15),
    t("taxes", Money, 15),
    t("taxation", Money, 15),
    t("profit*", Money, 15),
    t("revenue", Money, 15),
    t("loan*", Money, 15),
    t("mortgage*", Money, 15),
    t("stock", Money, 10),
    t("stocks", Money, 10),
    t("market", Money, 10),
    t("markets", Money, 10),
    t("afford*", Money, 15),
    t("cheap*", Money, 15),
    t("expensive", Money, 15),
    t("cost", Money, 15),
    t("costs", Money, 15),
    t("costly", Money, 15),
    t("pay", Money, 10),
    t("pays", Money, 10),
    t("paid", Money, 10),
    t("payment*", Money, 10),
    t("paycheck*", Money, 15),
    t("earn*", Money, 10),
    t("rich", Money, 10),
    t("wealth*", Money, 15),
    t("dollar*", Money, 20),
    t("rupee*", Money, 20),
    t("euro", Money, 20),
    t("euros", Money, 20),
    t("cents", Money, 10),
    t("rent", Money, 10),
    t("receipt*", Money, 10),
    t("grocer*", Money, 10),
    t("business*", Money, 10),
    t("customer*", Money, 5),
    t("sales", Money, 10),
    t("sell", Money, 5),
    t("sells", Money, 5),
    t("spend*", Money, 10),
    t("return", Money, 5),
    t("returns", Money, 5),
    t("$", Money, 5),
    t("€", Money, 5),
    t("£", Money, 5),
    t("₹", Money, 5),
    t("¥", Money, 5),
    // --- data words: weakly neutral
    t("percent*", Stat, 5),
    t("ratio", Stat, 5),
    t("average", Stat, 5),
    t("survey*", Stat, 5),
    t("statistic*", Stat, 5),
    t("study", Stat, 5),
    // --- rankings
    t("ranking*", Rank, 10),
    t("ranked", Rank, 10),
    t("rank", Rank, 10),
    t("leaderboard*", Rank, 15),
    t("top five", Rank, 10),
    t("top ten", Rank, 10),
    // --- losses and rising costs (money becomes serious)
    t("inflation", Loss, 20),
    t("debt*", Loss, 20),
    t("crisis", Loss, 20),
    t("crises", Loss, 20),
    t("recession*", Loss, 20),
    t("bankrupt*", Loss, 20),
    t("shortage*", Loss, 15),
    t("leak", Loss, 15),
    t("leaks", Loss, 15),
    t("leaking", Loss, 15),
    t("leakage", Loss, 15),
    t("cost* more", Loss, 20),
    t("earning less", Loss, 20),
    t("earn less", Loss, 20),
    t("lose money", Loss, 15),
    t("loss", Loss, 10),
    t("losses", Loss, 10),
    t("lose", Loss, 5),
    t("loses", Loss, 5),
    t("losing", Loss, 5),
    t("squeez*", Loss, 10),
    t("lost", Loss, 5),
    t("layoff*", Loss, 20),
    t("laid off", Loss, 20),
    t("fired", Loss, 15),
    t("unemploy*", Loss, 20),
    t("jobless*", Loss, 20),
    t("redundan*", Loss, 15),
    t("scam*", Loss, 15),
    t("fraud*", Loss, 15),
    t("struggl*", Loss, 10),
    t("declin*", Loss, 10),
    // --- work and office life: serious
    t("office", Work, 5),
    t("desk", Work, 5),
    t("meeting", Work, 5),
    t("meetings", Work, 5),
    t("overtime", Work, 15),
    t("workday", Work, 10),
    t("workplace", Work, 10),
    t("burnout", Work, 20),
    t("overwork*", Work, 15),
    t("career", Work, 5),
    t("job", Work, 5),
    t("jobs", Work, 5),
    t("looking for work", Loss, 15),
    // --- public affairs: serious
    t("government*", Civic, 10),
    t("justice", Civic, 10),
    t("polic*", Civic, 5),
    t("election*", Civic, 10),
    t("regulat*", Civic, 10),
    t("warn", Civic, 10),
    t("warns", Civic, 10),
    t("warning", Civic, 10),
    t("risk*", Civic, 10),
    t("investigat*", Civic, 10),
    // --- history: serious (wonder with discovery words)
    t("histor*", History, 10),
    t("century", History, 10),
    t("centuries", History, 10),
    t("empire*", History, 15),
    t("dynast*", History, 15),
    t("kingdom*", History, 10),
    t("medieval", History, 15),
    t("ancient", History, 15),
    t("civilization*", History, 10),
    t("revolution*", History, 10),
    t("colonial*", History, 10),
    t("monarch*", History, 10),
    t("pharaoh*", History, 15),
    t("emperor*", History, 15),
    t("reign*", History, 10),
    t("scroll*", History, 10),
    t("ruins", History, 10),
    t("pyramid*", History, 10),
    t("temple*", History, 5),
    t("discover*", Discovery, 10),
    t("uncover*", Discovery, 10),
    t("unearth*", Discovery, 10),
    t("archaeolog*", Discovery, 15),
    t("mystery", Discovery, 10),
    t("mysteries", Discovery, 10),
    t("mysterious", Discovery, 10),
    t("explor*", Discovery, 10),
    t("expedition*", Discovery, 10),
    // --- grief, hardship and war: somber
    t("death*", Grief, 20),
    t("the dead", Grief, 15),
    t("die", Grief, 10),
    t("dies", Grief, 10),
    t("died", Grief, 15),
    t("dying", Grief, 10),
    t("grief", Grief, 20),
    t("griev*", Grief, 20),
    t("mourn*", Grief, 20),
    t("funeral*", Grief, 15),
    t("tragedy", Grief, 20),
    t("tragic", Grief, 20),
    t("victim*", Grief, 20),
    t("toll", Hardship, 10),
    t("casualt*", Grief, 20),
    t("fatal*", Grief, 15),
    t("killed", Grief, 15),
    t("killing", Grief, 15),
    t("survivor*", Grief, 15),
    t("lives lost", Grief, 20),
    t("lost lives", Grief, 20),
    t("lonely", Hardship, 15),
    t("loneliness", Hardship, 15),
    t("alone", Hardship, 5),
    t("heartbreak*", Hardship, 10),
    t("suffer*", Hardship, 10),
    t("missing", Hardship, 5),
    t("homeless*", Hardship, 15),
    t("displaced", Hardship, 15),
    t("refugee*", Hardship, 20),
    t("famine", Hardship, 20),
    t("hunger", Hardship, 15),
    t("starv*", Hardship, 15),
    t("poverty", Hardship, 15),
    t("disaster*", Hardship, 20),
    t("flood", Hardship, 15),
    t("floods", Hardship, 15),
    t("flooded", Hardship, 15),
    t("flooding", Hardship, 15),
    t("devastat*", Hardship, 20),
    t("rubble", Hardship, 15),
    t("evacuat*", Hardship, 15),
    t("radiation", Hardship, 20),
    t("aftermath", Hardship, 15),
    t("rescue*", Hardship, 5),
    t("relief", Hardship, 5),
    t("lost their homes", Hardship, 20),
    t("lost homes", Hardship, 20),
    t("no roof", Hardship, 20),
    t("homes destroyed", Hardship, 20),
    t("destroyed homes", Hardship, 20),
    t("without rest", Hardship, 15),
    // (coordinator, review escalation) displacement told without a
    // disaster noun: a town that was left and never came back.
    t("never came back", Grief, 20),
    t("never returned", Grief, 20),
    t("exclusion zone", Hardship, 15),
    t("pandemic*", Hardship, 20),
    t("epidemic*", Hardship, 20),
    t("disease*", Hardship, 20),
    t("plague*", Hardship, 15),
    t("outbreak*", Hardship, 15),
    t("cancer", Hardship, 15),
    t("illness*", Hardship, 10),
    t("covid", Hardship, 15),
    t("crash", Hardship, 10),
    t("crashes", Hardship, 10),
    t("crashed", Hardship, 10),
    t("pollut*", Hardship, 10),
    t("extinct*", Hardship, 15),
    t("endangered", Hardship, 15),
    t("exhaust*", Rest, 5),
    t("war", War, 15),
    t("wars", War, 15),
    t("conflict*", War, 5),
    t("genocide", Grief, 20),
    t("massacre*", Grief, 20),
    // --- space, science and nature: wonder
    t("space", Space, 10),
    t("planet*", Space, 20),
    t("universe*", Space, 20),
    t("galax*", Space, 20),
    t("cosmos", Space, 20),
    t("cosmic", Space, 20),
    t("astronom*", Space, 20),
    t("astronaut*", Space, 15),
    t("telescope*", Space, 20),
    t("orbit*", Space, 10),
    t("solar", Space, 15),
    t("moon", Space, 10),
    t("mars", Space, 15),
    t("jupiter", Space, 15),
    t("saturn", Space, 15),
    t("venus", Space, 10),
    t("neptune", Space, 15),
    t("earth", Space, 10),
    t("earths", Space, 10),
    t("star", Space, 5),
    t("stars", Space, 5),
    t("sun", Space, 5),
    t("sunlight", Space, 10),
    t("light years", Space, 20),
    t("black hole*", Space, 20),
    t("asteroid*", Space, 15),
    t("comet*", Space, 15),
    t("satellite*", Space, 10),
    t("eclipse*", Space, 15),
    t("gravity", Space, 10),
    t("atmospher*", Space, 15),
    t("ozone", Space, 15),
    t("tropo*", Space, 15),
    t("strato*", Space, 15),
    t("weather", Science, 10),
    t("climate", Science, 10),
    t("ocean*", Science, 20),
    t("deep sea", Science, 15),
    t("atom", Science, 20),
    t("atoms", Science, 20),
    t("atomic", Science, 20),
    t("molecul*", Science, 15),
    t("science", Science, 5),
    t("sciences", Science, 5),
    t("scientific", Science, 10),
    t("scientist*", Science, 10),
    t("physics", Science, 15),
    t("chemistry", Science, 15),
    t("biology", Science, 15),
    t("quantum", Science, 15),
    t("brain", Science, 5),
    t("brains", Science, 5),
    t("neuron*", Science, 15),
    t("cells", Science, 5),
    t("dna", Science, 15),
    t("genetic*", Science, 10),
    t("evolution", Science, 15),
    t("evolve*", Science, 15),
    t("species", Science, 10),
    t("nature", Science, 10),
    t("rainforest*", Science, 15),
    t("wildlife", Science, 10),
    t("ecosystem*", Science, 15),
    t("photosynthesis", Science, 20),
    t("oxygen", Science, 10),
    t("plankton*", Science, 15),
    t("photic", Science, 15),
    t("aphotic", Science, 15),
    t("thermocline", Science, 15),
    t("pycnocline", Science, 15),
    t("stratification", Science, 10),
    t("immune", Science, 10),
    t("immunity", Science, 10),
    t("antibod*", Science, 10),
    t("vaccin*", Science, 10),
    t("microscop*", Science, 10),
    t("bacteria", Science, 10),
    t("organism*", Science, 10),
    t("dinosaur*", Science, 15),
    t("fossil*", Science, 15),
    t("million years", Science, 15),
    t("billion years", Science, 15),
    t("wonder", Science, 10),
    t("wonders", Science, 10),
    // --- celebration and lift: upbeat
    t("celebrat*", Celebration, 15),
    t("party", Celebration, 15),
    t("parties", Celebration, 15),
    t("cheer*", Celebration, 15),
    t("joy", Celebration, 15),
    t("joyful", Celebration, 15),
    t("happy", Celebration, 15),
    t("happiness", Celebration, 15),
    t("good news", Celebration, 15),
    t("great news", Celebration, 15),
    t("amazing", Celebration, 10),
    t("awesome", Celebration, 15),
    t("exciting", Celebration, 15),
    t("excited", Celebration, 15),
    t("excitement", Celebration, 15),
    t("fantastic", Celebration, 10),
    t("wow", Celebration, 10),
    t("festival*", Celebration, 15),
    t("dance*", Celebration, 15),
    t("holiday*", Celebration, 10),
    t("adventure*", Celebration, 10),
    t("boost*", Boost, 10),
    t("supercharg*", Boost, 15),
    t("level up", Boost, 15),
    t("productiv*", Boost, 10),
    t("upgrade*", Boost, 10),
    t("faster", Boost, 5),
    t("ten times", Boost, 10),
    t("10x", Boost, 10),
    // --- surprise, trivia and retro culture: playful
    t("wait what", Trivia, 40),
    t("fun fact*", Trivia, 30),
    t("did you know", Trivia, 30),
    t("trivia", Trivia, 30),
    t("believe it or not", Trivia, 30),
    t("fun", Trivia, 15),
    t("funny", Trivia, 20),
    t("weird*", Trivia, 20),
    t("surpris*", Trivia, 15),
    t("technically", Trivia, 10),
    t("crazy", Trivia, 15),
    t("crazi*", Trivia, 15),
    t("strang*", Trivia, 10),
    t("bizarre", Trivia, 15),
    t("silly", Trivia, 20),
    t("joke*", Trivia, 20),
    t("meme*", Trivia, 20),
    t("cute", Trivia, 15),
    t("lol", Trivia, 15),
    t("oops", Trivia, 15),
    t("wacky", Trivia, 20),
    t("myth*", Trivia, 10),
    t("quiz*", Trivia, 20),
    t("riddle*", Trivia, 20),
    t("puzzle*", Trivia, 15),
    t("game", Trivia, 10),
    t("games", Trivia, 10),
    t("toy", Trivia, 10),
    t("toys", Trivia, 10),
    t("prank*", Trivia, 20),
    t("giggle*", Trivia, 20),
    t("hilarious", Trivia, 20),
    t("guess*", Trivia, 10),
    t("fact", Trivia, 5),
    t("facts", Trivia, 5),
    t("retro", Retro, 20),
    t("vintage", Retro, 15),
    t("nostalgi*", Retro, 20),
    t("arcade*", Retro, 20),
    t("cassette*", Retro, 15),
    t("mixtape*", Retro, 15),
    t("vinyl", Retro, 15),
    t("tv", Retro, 10),
    t("television*", Retro, 10),
    t("megaphone*", Retro, 10),
    t("rotary", Retro, 15),
    t("disco", Retro, 20),
    t("jukebox*", Retro, 20),
    t("cartoon*", Retro, 15),
    t("song", Retro, 5),
    t("songs", Retro, 5),
    t("taped", Retro, 5),
    // --- force and spectacle: dramatic
    t("epic", Spectacle, 20),
    t("storm*", Spectacle, 15),
    t("erupt*", Spectacle, 25),
    t("volcan*", Spectacle, 20),
    t("lava", Spectacle, 20),
    t("magma", Spectacle, 20),
    t("launch*", Spectacle, 10),
    t("liftoff", Spectacle, 15),
    t("lift off", Spectacle, 15),
    t("rocket*", Spectacle, 10),
    t("battle*", Spectacle, 20),
    t("explod*", Spectacle, 15),
    t("explos*", Spectacle, 15),
    t("earthquake*", Spectacle, 10),
    t("tsunami*", Spectacle, 15),
    t("hurricane*", Spectacle, 15),
    t("tornado*", Spectacle, 15),
    t("wildfire*", Spectacle, 15),
    t("thunder*", Spectacle, 15),
    t("lightning", Spectacle, 15),
    t("blast*", Spectacle, 10),
    t("collision*", Spectacle, 10),
    t("titan*", Spectacle, 10),
    t("legend*", Spectacle, 10),
    t("hero", Spectacle, 10),
    t("heroes", Spectacle, 10),
    t("heroic", Spectacle, 10),
    t("fury", Spectacle, 15),
    t("unleash*", Spectacle, 20),
    t("inferno", Spectacle, 20),
    t("avalanche*", Spectacle, 20),
    t("apocalyp*", Spectacle, 20),
    t("colossal", Spectacle, 10),
    t("raging", Spectacle, 15),
    t("destroy*", Spectacle, 15),
    t("destruction", Spectacle, 15),
    t("catastroph*", Spectacle, 15),
    t("thrilling", Spectacle, 15),
    t("breathtaking", Spectacle, 15),
    t("meteor*", Spectacle, 5),
    t("supernova*", Spectacle, 15),
    // --- deadlines and ticking clocks: tense
    t("deadline*", Deadline, 25),
    t("countdown*", Deadline, 25),
    t("seconds left", Deadline, 25),
    t("seconds remain*", Deadline, 25),
    t("minutes left", Deadline, 20),
    t("hours left", Deadline, 15),
    t("days left", Deadline, 15),
    t("t minus", Deadline, 20),
    t("ticking", Deadline, 25),
    t("race", Deadline, 10),
    t("racing", Deadline, 15),
    t("race against", Deadline, 20),
    t("urgent*", Deadline, 20),
    t("emergency", Deadline, 20),
    t("hurry*", Deadline, 15),
    t("rush*", Deadline, 10),
    t("panic*", Deadline, 15),
    t("suspense*", Deadline, 20),
    t("tension*", Deadline, 20),
    t("tense", Deadline, 20),
    t("standoff", Deadline, 20),
    t("final seconds", Deadline, 20),
    t("last chance", Deadline, 20),
    t("too late", Deadline, 15),
    t("running out of time", Deadline, 20),
    t("now or never", Deadline, 25),
    t("ignition", Deadline, 15),
    t("stakes", Deadline, 10),
    t("chase*", Deadline, 15),
    t("danger*", Deadline, 5),
    t("threat*", Deadline, 5),
    // --- rest and quiet: calm
    t("sleep*", Rest, 15),
    t("nap", Rest, 15),
    t("naps", Rest, 15),
    t("napping", Rest, 15),
    t("rest", Rest, 5),
    t("resting", Rest, 10),
    t("restful", Rest, 15),
    t("calm*", Rest, 15),
    t("quiet*", Rest, 15),
    t("peace*", Rest, 15),
    t("relax*", Rest, 15),
    t("gentl*", Rest, 10),
    t("breathe", Rest, 10),
    t("breath", Rest, 10),
    t("breathing", Rest, 10),
    t("mindful*", Rest, 15),
    t("meditat*", Rest, 20),
    t("silence", Rest, 15),
    t("silent", Rest, 15),
    t("serene", Rest, 15),
    t("tranquil", Rest, 15),
    t("cozy", Rest, 15),
    t("soothing", Rest, 15),
    t("lullaby", Rest, 15),
    t("bedtime", Rest, 15),
    t("stillness", Rest, 15),
    t("focus*", Rest, 10),
    t("reflect*", Rest, 10),
    t("wellbeing", Rest, 10),
    t("stress*", Rest, 10),
    t("slump", Rest, 5),
    t("groggy", Rest, 5),
    // --- habits, goals and courage: inspiring
    t("habit*", Habits, 15),
    t("goal*", Habits, 15),
    t("fear", Habits, 15),
    t("fears", Habits, 15),
    t("fearless", Habits, 15),
    t("scared", Habits, 10),
    t("courage*", Habits, 20),
    t("brave*", Habits, 15),
    t("first step", Habits, 25),
    t("start", Habits, 10),
    t("starts", Habits, 10),
    t("started", Habits, 10),
    t("starting", Habits, 10),
    t("begin*", Habits, 10),
    t("train*", Habits, 10),
    t("practice*", Habits, 10),
    t("finish line", Habits, 15),
    t("step by step", Habits, 15),
    t("little by little", Habits, 15),
    t("day by day", Habits, 10),
    t("skill*", Habits, 10),
    t("athlete*", Habits, 5),
    t("grow", Habits, 10),
    t("grows", Habits, 10),
    t("growth", Habits, 10),
    t("you can", Habits, 10),
    t("believe*", Habits, 15),
    t("dream*", Habits, 15),
    t("never give up", Habits, 25),
    t("keep going", Habits, 25),
    t("persever*", Habits, 20),
    t("resilien*", Habits, 20),
    t("success*", Habits, 15),
    t("achiev*", Habits, 15),
    t("win", Habits, 10),
    t("wins", Habits, 10),
    t("winning", Habits, 10),
    t("winner*", Habits, 10),
    t("victory", Habits, 10),
    t("victories", Habits, 10),
    t("trophy", Habits, 10),
    t("trophies", Habits, 10),
    t("progress*", Habits, 10),
    t("improv*", Habits, 10),
    t("consisten*", Habits, 15),
    t("patience", Habits, 10),
    t("idea", Habits, 10),
    t("ideas", Habits, 10),
    t("move on", Habits, 15),
    t("freedom", Habits, 10),
    t("trust", Habits, 5),
    t("promise*", Habits, 5),
    t("motivat*", Habits, 20),
    t("inspir*", Habits, 20),
    t("potential", Habits, 15),
    t("transform*", Habits, 10),
    t("journey", Habits, 10),
    t("mindset", Habits, 15),
    t("confiden*", Habits, 5),
    t("purpose", Habits, 10),
    t("passion*", Habits, 15),
    t("overcome", Habits, 20),
    t("empower*", Habits, 20),
    t("unstoppable", Habits, 20),
];

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn is_currency(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '₹' | '¥')
}

/// Lowercase word tokens (letters and digits); apostrophes inside a word are
/// dropped; a currency sign is a token of its own.
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            cur.extend(ch.to_lowercase());
        } else if ch == '\'' || ch == '\u{2019}' {
            // "that's" -> "thats"
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if is_currency(ch) {
                out.push(ch.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn push_text(out: &mut Vec<Vec<String>>, text: Option<&str>) {
    if let Some(s) = text {
        let toks = tokenize(s);
        if !toks.is_empty() {
            out.push(toks);
        }
    }
}

fn collect_atom(out: &mut Vec<Vec<String>>, value: Option<&str>, meaning: Option<&str>) {
    push_text(out, value);
    push_text(out, meaning);
}

fn collect_subject(out: &mut Vec<Vec<String>>, s: &Subject) {
    match s {
        Subject::Phrase(a) | Subject::Number(a) => {
            collect_atom(out, a.value.as_deref(), a.meaning.as_deref())
        }
        Subject::Object(o) => collect_atom(out, o.value.as_deref(), o.meaning.as_deref()),
        Subject::Collection(c) => {
            push_text(out, c.meaning.as_deref());
            for item in &c.items {
                match item {
                    CollectionItem::Phrase(a) | CollectionItem::Number(a) => {
                        collect_atom(out, a.value.as_deref(), a.meaning.as_deref())
                    }
                    CollectionItem::Object(o) => {
                        collect_atom(out, o.value.as_deref(), o.meaning.as_deref())
                    }
                }
            }
        }
        Subject::StateChange(c) => {
            push_text(out, Some(&c.entity));
            push_text(out, Some(&c.from));
            push_text(out, Some(&c.to));
            push_text(out, c.meaning.as_deref());
        }
        Subject::DerivedMetric(m) => {
            push_text(out, m.meaning.as_deref());
            push_text(out, Some(&m.numerator.meaning));
            push_text(out, Some(&m.denominator.meaning));
        }
        Subject::Layers(l) => {
            push_text(out, l.meaning.as_deref());
            for layer in &l.layers {
                push_text(out, Some(&layer.name));
                push_text(out, layer.note.as_deref());
            }
        }
    }
}

/// Every text field of the story, tokenised separately (a phrase never
/// matches across two fields).
fn story_fields(intent: &CreativeIntent) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for beat in &intent.beats {
        push_text(&mut out, beat.narration.as_deref());
        push_text(&mut out, Some(&beat.statement));
        push_text(&mut out, beat.keyword.as_deref());
        collect_subject(&mut out, &beat.primary);
        if let Some(s) = &beat.secondary {
            collect_subject(&mut out, s);
        }
    }
    out
}

/// A lexicon pattern split into tokens once: (word or prefix, is prefix).
struct Compiled {
    toks: Vec<(&'static str, bool)>,
    group: Group,
    points: i32,
}

fn compiled() -> &'static [Compiled] {
    static LEX: OnceLock<Vec<Compiled>> = OnceLock::new();
    LEX.get_or_init(|| {
        LEXICON
            .iter()
            .filter(|t| t.points > 0)
            .map(|t| Compiled {
                toks: t
                    .pat
                    .split(' ')
                    .map(|p| match p.strip_suffix('*') {
                        Some(prefix) => (prefix, true),
                        None => (p, false),
                    })
                    .collect(),
                group: t.group,
                points: t.points,
            })
            .collect()
    })
}

fn matches_at(tokens: &[String], pat: &[(&str, bool)]) -> bool {
    pat.len() <= tokens.len()
        && pat
            .iter()
            .zip(tokens)
            .all(|(&(p, prefix), tok)| if prefix { tok.starts_with(p) } else { tok == p })
}

fn occurrences(fields: &[Vec<String>], pat: &[(&str, bool)]) -> usize {
    fields
        .iter()
        .map(|toks| {
            if toks.len() < pat.len() {
                return 0;
            }
            (0..=toks.len() - pat.len())
                .filter(|&i| matches_at(&toks[i..], pat))
                .count()
        })
        .sum()
}

// ---------------------------------------------------------------------------
// Structure
// ---------------------------------------------------------------------------

fn has_digit(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.chars().any(|c| c.is_ascii_digit()))
}

/// A list of three or more items that carry figures (a ranking or a table).
fn is_figure_list(s: &Subject) -> bool {
    let Subject::Collection(c) = s else {
        return false;
    };
    let figures = c
        .items
        .iter()
        .filter(|i| match i {
            CollectionItem::Phrase(a) | CollectionItem::Number(a) => has_digit(a.value.as_deref()),
            CollectionItem::Object(o) => has_digit(o.value.as_deref()),
        })
        .count();
    c.items.len() >= 3 && figures >= 3
}

/// A subject that is itself a figure.
fn is_figure(s: &Subject) -> bool {
    match s {
        Subject::Number(_) | Subject::DerivedMetric(_) | Subject::StateChange(_) => true,
        Subject::Object(o) => has_digit(o.value.as_deref()),
        other => is_figure_list(other),
    }
}

/// (ranking points, number-led-run points).
fn structure_points(intent: &CreativeIntent) -> (i32, i32) {
    let ranking = intent
        .beats
        .iter()
        .any(|b| is_figure_list(&b.primary) || b.secondary.as_ref().is_some_and(is_figure_list));
    let figure_beats = intent
        .beats
        .iter()
        .filter(|b| is_figure(&b.primary) || b.secondary.as_ref().is_some_and(is_figure))
        .count();
    let numeric_run = figure_beats >= 3 && figure_beats * 2 >= intent.beats.len();
    (
        if ranking { 10 } else { 0 },
        if numeric_run { 5 } else { 0 },
    )
}

// ---------------------------------------------------------------------------
// Energy
// ---------------------------------------------------------------------------

fn beat_energy(e: Energy) -> usize {
    match e {
        Energy::Calm => 1,
        Energy::Building => 3,
        Energy::Impact => 5,
    }
}

/// The rounded mean of the beats' energies (calm 1, building 3, impact 5).
fn spread_energy(intent: &CreativeIntent) -> u8 {
    let n = intent.beats.len();
    if n == 0 {
        return 3;
    }
    let sum: usize = intent.beats.iter().map(|b| beat_energy(b.energy)).sum();
    (((2 * sum + n) / (2 * n)) as u8).clamp(1, 5)
}

/// Energy 5 for a hype or energetic look, 1 for a documentary or restrained
/// look, else the spread of the beats' own energy.
fn tone_energy(taste: &ResolvedStyleProfile, spread: u8) -> u8 {
    let high = taste.genre == Genre::Hype || taste.motion.kind == TemperamentKind::Energetic;
    let low = taste.genre == Genre::Documentary || taste.motion.kind == TemperamentKind::Restrained;
    match (high, low) {
        (true, false) => 5,
        (false, true) => 1,
        _ => spread,
    }
}

// ---------------------------------------------------------------------------
// Story mood
// ---------------------------------------------------------------------------

fn family_index(f: MoodFamily) -> usize {
    MoodFamily::ALL.iter().position(|&x| x == f).unwrap_or(0)
}

/// The families a tone leans to (first, second), for ambiguous content only.
fn tone_lean(taste: &ResolvedStyleProfile) -> Option<(MoodFamily, Option<MoodFamily>)> {
    use MoodFamily::*;
    match taste.genre {
        Genre::Hype => Some((Upbeat, Some(Tense))),
        Genre::Documentary => Some((NeutralExplainer, Some(Serious))),
        Genre::Cinematic => Some((Wonder, Some(Dramatic))),
        Genre::Studio => Some((Upbeat, Some(Inspiring))),
        Genre::Street => None,
        Genre::Standard => match taste.tone {
            ResolvedTone::Playful => Some((Playful, None)),
            ResolvedTone::Editorial if taste.motion.kind == TemperamentKind::Restrained => {
                Some((Calm, None))
            }
            _ => None,
        },
    }
}

/// One scored reading of a story: the points per family and where they came
/// from. Public so tests and tools can show the margins.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodScores {
    /// Points per family, in [`MoodFamily::ALL`] order.
    pub points: Vec<(MoodFamily, i32)>,
    /// The winner.
    pub family: MoodFamily,
    /// The reason of the winner ("money story", "no strong signal").
    pub reason: String,
}

fn bump(by: &mut Vec<(Group, i32)>, g: Group, p: i32) {
    if p == 0 {
        return;
    }
    match by.iter_mut().find(|(x, _)| *x == g) {
        Some(e) => e.1 += p,
        None => by.push((g, p)),
    }
}

fn group_points(by: &[(Group, i32)], g: Group) -> i32 {
    by.iter().find(|(x, _)| *x == g).map_or(0, |e| e.1)
}

/// Score a story. See the module docs for the rules.
pub fn score_story(intent: &CreativeIntent, taste: &ResolvedStyleProfile) -> MoodScores {
    let fields = story_fields(intent);
    let mut by_group: Vec<(Group, i32)> = Vec::new();
    for term in compiled() {
        let n = occurrences(&fields, &term.toks).min(TERM_CAP) as i32;
        bump(&mut by_group, term.group, term.points * n);
    }
    let (ranking, numeric_run) = structure_points(intent);
    bump(&mut by_group, Group::Rank, ranking);
    bump(&mut by_group, Group::Stat, numeric_run);
    let get = |g: Group| group_points(&by_group, g);

    // contributions: (family, group named in the reason, points)
    let mut contrib: Vec<(MoodFamily, Group, i32)> = Vec::new();
    let mut add = |f: MoodFamily, g: Group, p: i32| {
        if p > 0 {
            contrib.push((f, g, p));
        }
    };
    let loss = get(Group::Loss);
    let money = get(Group::Money);
    let history = get(Group::History);
    let discovery = get(Group::Discovery);
    let grief = get(Group::Grief);
    let hardship = get(Group::Hardship);

    // Money: a neutral explainer, serious when loss words are present.
    add(MoodFamily::Serious, Group::Loss, loss);
    if loss > 0 {
        add(MoodFamily::Serious, Group::Loss, money);
    } else {
        add(MoodFamily::NeutralExplainer, Group::Money, money);
    }
    add(MoodFamily::NeutralExplainer, Group::Stat, get(Group::Stat));
    add(MoodFamily::NeutralExplainer, Group::Rank, get(Group::Rank));
    add(MoodFamily::Serious, Group::Work, get(Group::Work));
    add(MoodFamily::Serious, Group::Civic, get(Group::Civic));
    // History: serious, wonder with discovery words.
    add(MoodFamily::Wonder, Group::Discovery, discovery);
    if discovery >= 10 {
        add(MoodFamily::Wonder, Group::History, history);
    } else {
        add(MoodFamily::Serious, Group::History, history);
    }
    // Grief and hardship: somber. War: serious in a history story without
    // grief words, somber otherwise.
    add(MoodFamily::Somber, Group::Grief, grief);
    add(MoodFamily::Somber, Group::Hardship, hardship);
    // Company rules (module docs, step 3). Any death word (grief points of at
    // least GRIEF_COMPANY) or enough hardship turns rest words into mourning
    // (somber); grief plus hardship of at least HARDSHIP_COMPANY makes spectacle
    // words describe a disaster, so half their points go to serious.
    let rest_to_somber = grief >= GRIEF_COMPANY || hardship >= HARDSHIP_COMPANY;
    let spectacle_to_serious = grief + hardship >= HARDSHIP_COMPANY;
    if rest_to_somber {
        add(MoodFamily::Somber, Group::Hardship, get(Group::Rest));
    } else {
        add(MoodFamily::Calm, Group::Rest, get(Group::Rest));
    }
    if spectacle_to_serious {
        add(
            MoodFamily::Serious,
            Group::Hardship,
            get(Group::Spectacle) / 2,
        );
    } else {
        add(
            MoodFamily::Dramatic,
            Group::Spectacle,
            get(Group::Spectacle),
        );
    }
    if history >= 10 && grief == 0 {
        add(MoodFamily::Serious, Group::War, get(Group::War));
    } else {
        add(MoodFamily::Somber, Group::War, get(Group::War));
    }
    add(MoodFamily::Wonder, Group::Science, get(Group::Science));
    add(MoodFamily::Wonder, Group::Space, get(Group::Space));
    add(
        MoodFamily::Upbeat,
        Group::Celebration,
        get(Group::Celebration),
    );
    add(MoodFamily::Upbeat, Group::Boost, get(Group::Boost));
    add(MoodFamily::Playful, Group::Trivia, get(Group::Trivia));
    add(MoodFamily::Playful, Group::Retro, get(Group::Retro));
    add(MoodFamily::Tense, Group::Deadline, get(Group::Deadline));
    add(MoodFamily::Inspiring, Group::Habits, get(Group::Habits));

    let mut points = vec![0i32; MoodFamily::ALL.len()];
    points[family_index(MoodFamily::NeutralExplainer)] += NEUTRAL_PRIOR;
    for &(f, _, p) in &contrib {
        points[family_index(f)] += p;
    }
    if let Some((first, second)) = tone_lean(taste) {
        points[family_index(first)] += TONE_FIRST;
        if let Some(s) = second {
            points[family_index(s)] += TONE_SECOND;
        }
    }

    // The winner: most points; the earlier family (neutral first) wins ties.
    let mut best = 0usize;
    for (i, &p) in points.iter().enumerate() {
        if p > points[best] {
            best = i;
        }
    }
    let family = MoodFamily::ALL[best];
    let mut top: Option<(Group, i32)> = None;
    for &(f, g, p) in &contrib {
        if f == family && top.is_none_or(|(_, tp)| p > tp) {
            top = Some((g, p));
        }
    }
    let reason = top.map_or_else(
        || "no strong signal".to_string(),
        |(g, _)| g.label().to_string(),
    );
    MoodScores {
        points: MoodFamily::ALL.iter().copied().zip(points).collect(),
        family,
        reason,
    }
}

/// The body of [`crate::audio::story_mood`].
pub fn read_mood(intent: &CreativeIntent, taste: &ResolvedStyleProfile) -> MoodProfile {
    let scores = score_story(intent, taste);
    MoodProfile {
        family: scores.family,
        compatible: compatible_families(scores.family),
        energy: tone_energy(taste, spread_energy(intent)),
        reason: scores.reason,
    }
}

// ---------------------------------------------------------------------------
// Bed selection
// ---------------------------------------------------------------------------

fn bed_energy(b: &CatalogBed) -> i32 {
    if b.energy == 0 {
        3
    } else {
        i32::from(b.energy)
    }
}

/// Whether a bed may be used for content of `family` at all.
fn allowed_for(bed: &CatalogBed, family: MoodFamily) -> bool {
    if bed.moods.is_empty() {
        return false;
    }
    bed.id != DRAMA_ONLY_BED || matches!(family, MoodFamily::Dramatic | MoodFamily::Wonder)
}

/// The body of [`crate::audio::select_bed`], with the energy tie band as a
/// parameter (`select_bed` passes [`ENERGY_TIE_BAND`]).
pub fn select_bed_with_tie_band(
    catalog: &MusicCatalog,
    mood: &MoodProfile,
    word: MusicWord,
    seed: u64,
    tie_band: u8,
) -> BedChoice {
    if word == MusicWord::None {
        return BedChoice {
            bed: None,
            mood: mood.family,
            reason: "no music requested".into(),
            warning: None,
        };
    }
    let forced = forced_family(word);
    let family = forced.unwrap_or(mood.family);
    let compat = match forced {
        Some(f) => compatible_families(f),
        None if mood.compatible.is_empty() => compatible_families(family),
        None => mood.compatible.clone(),
    };
    let warning = forced.filter(|f| !mood.compatible.contains(f)).map(|f| {
        format!(
            "the music word '{}' asks for {} music, but this story reads {}; using {} music as asked",
            word.as_str(),
            family_label(f),
            family_label(mood.family),
            family_label(f),
        )
    });

    let eligible: Vec<&CatalogBed> = catalog
        .beds
        .iter()
        .filter(|b| allowed_for(b, family))
        .collect();
    let rank1: Vec<&CatalogBed> = eligible
        .iter()
        .copied()
        .filter(|b| b.moods.contains(&family))
        .collect();
    let (pool, rank) = if !rank1.is_empty() {
        (rank1, 1u8)
    } else {
        let rank2: Vec<&CatalogBed> = eligible
            .iter()
            .copied()
            .filter(|b| b.moods.first().is_some_and(|m| compat.contains(m)))
            .collect();
        (rank2, 2u8)
    };
    if pool.is_empty() {
        return BedChoice {
            bed: None,
            mood: family,
            reason: format!("no bed fits a {} story; silence", family_label(family)),
            warning,
        };
    }

    // Energy: closest to the story's energy (a hype / energetic look reads as
    // 5 and a documentary / restrained look as 1, see the module docs).
    let target = i32::from(mood.energy.clamp(1, 5));
    let distance = |b: &CatalogBed| (bed_energy(b) - target).abs();
    let closest = pool.iter().map(|b| distance(b)).min().unwrap_or(0);
    let mut tied: Vec<&CatalogBed> = pool
        .into_iter()
        .filter(|b| distance(b) <= closest + i32::from(tie_band))
        .collect();
    tied.sort_by(|a, b| a.id.cmp(&b.id));
    let pick = (choice(seed, 0, Dim::Bed) % tied.len() as u64) as usize;
    let bed = tied[pick];

    let reason = if forced.is_some() {
        format!("{} ({}, as asked)", bed.id, family_label(family))
    } else if rank == 1 {
        format!("{} ({}, {})", bed.id, family_label(family), mood.reason)
    } else {
        format!(
            "{} ({}, {}; closest fit)",
            bed.id,
            family_label(family),
            mood.reason
        )
    };
    BedChoice {
        bed: Some(bed.id.clone()),
        mood: family,
        reason,
        warning,
    }
}

/// The body of [`crate::audio::select_bed`].
pub fn choose_bed(
    catalog: &MusicCatalog,
    mood: &MoodProfile,
    word: MusicWord,
    seed: u64,
) -> BedChoice {
    select_bed_with_tie_band(catalog, mood, word, seed, ENERGY_TIE_BAND)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One word counts once: no two single-word patterns match the same word,
    /// no pattern repeats, and every pattern is lowercase and well formed.
    #[test]
    fn lexicon_patterns_are_well_formed_and_do_not_overlap() {
        let mut problems = Vec::new();
        for (i, a) in LEXICON.iter().enumerate() {
            if a.points < 0 || a.pat.is_empty() || a.pat != a.pat.to_lowercase() {
                problems.push(format!("malformed: {:?}", a.pat));
            }
            if a.pat.split(' ').any(|p| p.is_empty() || p == "*") {
                problems.push(format!("empty token: {:?}", a.pat));
            }
            for b in &LEXICON[i + 1..] {
                if a.pat == b.pat {
                    problems.push(format!("repeated: {:?}", a.pat));
                    continue;
                }
                if a.pat.contains(' ') || b.pat.contains(' ') {
                    continue;
                }
                let (ap, bp) = (a.pat.trim_end_matches('*'), b.pat.trim_end_matches('*'));
                let (a_prefix, b_prefix) = (a.pat.ends_with('*'), b.pat.ends_with('*'));
                let overlap = match (a_prefix, b_prefix) {
                    (false, false) => false,
                    (true, false) => bp.starts_with(ap),
                    (false, true) => ap.starts_with(bp),
                    (true, true) => ap.starts_with(bp) || bp.starts_with(ap),
                };
                if overlap {
                    problems.push(format!("overlap: {:?} and {:?}", a.pat, b.pat));
                }
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn tokens_are_lowercase_words_and_currency_signs() {
        assert_eq!(
            tokenize("That's a $5 deal, Wait... WHAT?"),
            ["thats", "a", "$", "5", "deal", "wait", "what"]
        );
        assert_eq!(
            tokenize("sun-lit \u{20b9}1,800"),
            ["sun", "lit", "\u{20b9}", "1", "800"]
        );
    }

    #[test]
    fn phrases_match_whole_token_runs_and_prefixes_match_stems() {
        let toks = vec![tokenize("it costs more and more, so wait what")];
        let count = |pat: &str| {
            let p: Vec<(&str, bool)> = pat
                .split(' ')
                .map(|w| match w.strip_suffix('*') {
                    Some(prefix) => (prefix, true),
                    None => (w, false),
                })
                .collect();
            occurrences(&toks, &p)
        };
        assert_eq!(count("cost* more"), 1);
        assert_eq!(count("wait what"), 1);
        assert_eq!(count("more"), 2);
        assert_eq!(count("cost"), 0, "an exact word is not a prefix");
        assert_eq!(count("what wait"), 0);
    }

    #[test]
    fn energy_is_the_rounded_mean_of_the_beats() {
        let beat = |e: &str| {
            format!(
                r#"{{"purpose":"emphasize","statement":"s","primary":{{"kind":"phrase","value":"v"}},"energy":"{e}"}}"#
            )
        };
        let intent = |energies: &[&str]| {
            let beats: Vec<String> = energies.iter().map(|e| beat(e)).collect();
            let doc = format!(
                r#"{{"version":"0.2","title":"t","beats":[{}]}}"#,
                beats.join(",")
            );
            CreativeIntent::from_json(&doc).map_err(|e| e.to_string())
        };
        let energy = |energies: &[&str]| intent(energies).map(|i| spread_energy(&i));
        assert_eq!(energy(&["calm"]), Ok(1));
        assert_eq!(energy(&["building"]), Ok(3));
        assert_eq!(energy(&["impact"]), Ok(5));
        // (1 + 3 + 5 + 5) / 4 = 3.5 rounds up.
        assert_eq!(energy(&["calm", "building", "impact", "impact"]), Ok(4));
        assert_eq!(energy(&["calm", "building", "building", "impact"]), Ok(3));
    }
}
