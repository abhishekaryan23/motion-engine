//! PhotoStack: engraving and classical editorial cutouts with hand-painted
//! accent strokes (`Path` + `Trim`), scale variety (single hero → pair →
//! 3×3 grid → zoetrope ring), radial blueprints, 2.5D/3D extruded brand
//! marks and serif word-pair morphs (`ask` → `task`, `draft` → `craft`).

use motion_core::easing::Easing;
use motion_core::scene::{
    Asset, AssetKind, Color, ContactShadow, Fit, FontRole, ImageTreatment, Layer, LayerKind,
    MotionOp, PathFillRule, Scene, Stroke, TreatmentPreset,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lower::{layer, r6, scene, Ctx, Track, Use};
use crate::program::{Component, ComponentBody, MotionProgram, Swatch};
use crate::taste::{Shape, Spring};

/// Preset treatment and stroke aesthetic for a [`PhotoStack`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhotoPreset {
    /// High-contrast monochrome engraving/sculpture cutouts on light ground
    /// with hand-drawn ink & terracotta accent strokes (`Path` + `Trim`).
    #[default]
    EngravingRed,
    /// Clean editorial cutouts with bold framing rules.
    EditorialCut,
    /// Layered depth cutouts with parallax camera push.
    Parallax,
}

/// Visual layout grammar for one step of a [`PhotoStack`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhotoLayout {
    /// Geometric emblem mark + editorial serif wordmark, or radial rosette circle.
    BrandMark,
    /// Serif word-pair morph (`ask` → `task`, `draft` → `craft`, `Keep` → `thinking.`)
    /// with optional cutouts and ruled lines.
    WordMorph,
    /// Scale variety "many": 3×3 grid of engravings/cutouts with a marked centre cell.
    Grid,
    /// Circular 12-spoke zoetrope ring of silhouettes rotating around an accent dot.
    Zoetrope,
    /// Scale variety "big": single hero cutout with hand-drawn accent strokes on top.
    #[default]
    Hero,
    /// Radial geometric construction blueprint.
    Blueprint,
    /// Multi-layer 2.5D/3D extruded geometric emblem rotating in perspective.
    Starburst3d,
    /// Sparse star-field with constellation lines drawing to an accent pointer.
    Constellation,
    /// Woodcut circle-in-square geometric study with chords and sweeping arc.
    Vitruvian,
}

/// Hand-painted vector stroke overlay drawn on top of a [`PhotoStep`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccentStroke {
    /// Sweeping hand-drawn circle with an accent tip around a radial rosette.
    RosetteCircle,
    /// Top slash + exclamation stroke (`ask` → `ask !`).
    #[default]
    TopSlash,
    /// Calligraphic flourish header + baseline underline (`task`).
    Flourish,
    /// Horizontal red strike-through morphing the leading letter (`draft` → `craft`).
    StrikeMorph,
    /// Sweeping calligraphic arc over the subject's crest + bottom scale rule.
    NeckArc,
    /// Hand-drawn ink + accent oval circling the eye/head.
    EyeOval,
    /// Precision geometric target reticle + cardinal ticks over the focal point.
    StarburstStamp,
    /// Technical caliper bracket `[` with an accent locus dot.
    BracketDot,
    /// Double-ring anatomical circle + accent locus dot.
    RibCircle,
    /// Accent ink drop / period dot (`Keep` → `Keep thinking.`).
    PeriodDot,
}

impl AccentStroke {
    fn brush_spec(self, seed: u32) -> Option<motion_core::brush::BrushSpec> {
        use motion_core::brush::{BrushPreset, BrushSpec, PressureCurve};
        let (preset, pressure, wiggle) = match self {
            Self::EyeOval => (
                BrushPreset::Charcoal,
                PressureCurve::ThreePoint([0.65, 1.15, 0.55]),
                1.2,
            ),
            Self::NeckArc | Self::Flourish => (
                BrushPreset::Pen,
                PressureCurve::ThreePoint([0.55, 1.25, 0.45]),
                0.8,
            ),
            Self::TopSlash | Self::StrikeMorph => (
                BrushPreset::Marker2,
                PressureCurve::Linear([1.15, 0.75]),
                0.6,
            ),
            Self::RosetteCircle | Self::RibCircle | Self::BracketDot | Self::StarburstStamp => {
                (BrushPreset::Rotring, PressureCurve::Uniform, 0.3)
            }
            Self::PeriodDot => return None,
        };
        Some(BrushSpec {
            preset,
            weight: 1.0,
            pressure: Some(pressure),
            wiggle,
            seed,
            ..Default::default()
        })
    }
}

/// One fast cut or state inside a [`PhotoStack`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhotoStep {
    /// Seconds from the component's start (filled from `beat` or `beat_cuts`).
    #[serde(default)]
    pub at: f64,
    /// Music beat on which this step cuts in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    /// Composition layout for this step.
    #[serde(default)]
    pub layout: PhotoLayout,
    /// Primary cutout asset id (e.g. `"marble_bust"`, `"horse_head"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Multiple cutout asset ids (for `grid` or paired cutouts in `word_morph`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    /// Primary serif word or brand name (e.g. `"ask"`, `"draft"`, `"Studio"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Target word when morphing (`"task"`, `"craft"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_text: Option<String>,
    /// Secondary line revealed underneath (`"thinking."`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtext: Option<String>,
    /// Hand-drawn stroke overlay animated with `Trim`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<AccentStroke>,
    /// Draw horizontal ledger/notebook baselines across the frame.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ruled: bool,
}

/// An engraving / editorial cutout sequence with painted accent strokes,
/// word morphs and scale variety (`small → big → many`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhotoStack {
    #[serde(default)]
    pub preset: PhotoPreset,
    #[serde(default)]
    pub slot: crate::program::Slot,
    /// With music: advance to the next step every this many beats from
    /// `start_beat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_cuts: Option<u32>,
    /// Sequence of shots in time order (1..=32).
    pub steps: Vec<PhotoStep>,
}

/// Precise geometric metrics measured from PNG alpha channels for bundled cutouts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutoutMetrics {
    pub name: &'static str,
    pub id: &'static str,
    pub path: &'static str,
    pub dims: (u32, u32),
    pub alpha_bbox: [f32; 4],
    pub center_of_mass: [f32; 2],
    pub eye: Option<[f32; 2]>,
    pub focus: [f32; 3],         // [u, v, radius_ratio]
    pub halo_arc: [[f32; 2]; 3], // [start_uv, apex_uv, end_uv]
}

/// Exact verified cutout table measured from the PNG alpha channels.
pub const BUNDLED_CUTOUT_METRICS: [CutoutMetrics; 37] = [
    // classical_greyscale
    CutoutMetrics {
        name: "marble_bust",
        id: "cutout.marble_bust",
        path: "library/classical_greyscale/marble_bust.png",
        dims: (747, 1274),
        alpha_bbox: [0.0910, 0.0542, 0.9076, 0.9458],
        center_of_mass: [0.5214, 0.5135],
        eye: Some([0.6250, 0.2550]),
        focus: [0.5214, 0.2800, 0.2200],
        halo_arc: [[0.2352, 0.1127], [0.5783, 0.0249], [0.8344, 0.1127]],
    },
    CutoutMetrics {
        name: "bust_profile",
        id: "cutout.bust_profile",
        path: "library/classical_greyscale/bust_profile.png",
        dims: (673, 1245),
        alpha_bbox: [0.0996, 0.0538, 0.8990, 0.9446],
        center_of_mass: [0.5289, 0.4885],
        eye: Some([0.7370, 0.2297]),
        focus: [0.7370, 0.2297, 0.2000],
        halo_arc: [[0.1669, 0.1208], [0.6196, 0.0203], [0.9104, 0.1208]],
    },
    CutoutMetrics {
        name: "statue_eye",
        id: "cutout.statue_eye",
        path: "library/classical_greyscale/statue_eye.png",
        dims: (645, 855),
        alpha_bbox: [0.0713, 0.0538, 0.9271, 0.9450],
        center_of_mass: [0.4808, 0.4824],
        eye: Some([0.5395, 0.4982]),
        focus: [0.5395, 0.4982, 0.2500],
        halo_arc: [[0.0155, 0.1804], [0.2310, 0.0094], [0.9223, 0.1804]],
    },
    CutoutMetrics {
        name: "statue_torso",
        id: "cutout.statue_torso",
        path: "library/classical_greyscale/statue_torso.png",
        dims: (620, 1215),
        alpha_bbox: [0.1048, 0.0535, 0.8935, 0.9457],
        center_of_mass: [0.5145, 0.4826],
        eye: None,
        focus: [0.5000, 0.2500, 0.2200],
        halo_arc: [[0.0260, 0.1340], [0.4645, 0.0133], [0.9724, 0.1340]],
    },
    CutoutMetrics {
        name: "hand_holding",
        id: "cutout.hand_holding",
        path: "library/classical_greyscale/hand_holding.png",
        dims: (546, 821),
        alpha_bbox: [0.0806, 0.0536, 0.9176, 0.9452],
        center_of_mass: [0.4779, 0.4523],
        eye: None,
        focus: [0.4908, 0.2400, 0.2628],
        halo_arc: [[0.0200, 0.1600], [0.4800, 0.0090], [0.9800, 0.1600]],
    },
    CutoutMetrics {
        name: "hand_pointing",
        id: "cutout.hand_pointing",
        path: "library/classical_greyscale/hand_pointing.png",
        dims: (871, 512),
        alpha_bbox: [0.0540, 0.1230, 0.9449, 0.8730],
        center_of_mass: [0.4363, 0.4850],
        eye: None,
        focus: [0.8800, 0.2500, 0.1600],
        halo_arc: [[0.0100, 0.2200], [0.5050, 0.0070], [0.9900, 0.2200]],
    },
    CutoutMetrics {
        name: "hand_reaching",
        id: "cutout.hand_reaching",
        path: "library/classical_greyscale/hand_reaching.png",
        dims: (512, 1024),
        alpha_bbox: [0.2012, 0.0508, 0.7949, 0.9482],
        center_of_mass: [0.4539, 0.5107],
        eye: None,
        focus: [0.5200, 0.2800, 0.2200],
        halo_arc: [[0.1418, 0.1102], [0.6270, 0.0211], [0.8543, 0.1102]],
    },
    CutoutMetrics {
        name: "hands_touching",
        id: "cutout.hands_touching",
        path: "library/classical_greyscale/hands_touching.png",
        dims: (1792, 730),
        alpha_bbox: [0.0000, 0.1479, 0.9994, 0.8507],
        center_of_mass: [0.4971, 0.4596],
        eye: None,
        focus: [0.5089, 0.5205, 0.0500],
        halo_arc: [[0.0300, 0.2800], [0.5000, 0.0500], [0.9700, 0.1200]],
    },
    CutoutMetrics {
        name: "open_palm",
        id: "cutout.open_palm",
        path: "library/classical_greyscale/open_palm.png",
        dims: (755, 809),
        alpha_bbox: [0.0570, 0.0532, 0.9417, 0.9456],
        center_of_mass: [0.4620, 0.5156],
        eye: None,
        focus: [0.4620, 0.5156, 0.2200],
        halo_arc: [[0.0132, 0.1883], [0.2874, 0.0099], [0.7781, 0.1883]],
    },
    CutoutMetrics {
        name: "horse_head",
        id: "cutout.horse_head",
        path: "library/classical_greyscale/horse_head.png",
        dims: (708, 1022),
        alpha_bbox: [0.0777, 0.0528, 0.9209, 0.9452],
        center_of_mass: [0.4609, 0.5183],
        eye: Some([0.5720, 0.3376]),
        focus: [0.5720, 0.3376, 0.2000],
        halo_arc: [[0.0647, 0.1413], [0.5551, 0.0086], [0.8308, 0.1413]],
    },
    CutoutMetrics {
        name: "laurel",
        id: "cutout.laurel",
        path: "library/classical_greyscale/laurel.png",
        dims: (872, 840),
        alpha_bbox: [0.0539, 0.0560, 0.9450, 0.9429],
        center_of_mass: [0.5006, 0.5180],
        eye: None,
        focus: [0.5006, 0.4800, 0.2600],
        halo_arc: [[0.0400, 0.2200], [0.4323, 0.0040], [0.9500, 0.2200]],
    },
    CutoutMetrics {
        name: "scales",
        id: "cutout.scales",
        path: "library/classical_greyscale/scales.png",
        dims: (919, 977),
        alpha_bbox: [0.0566, 0.0542, 0.9423, 0.9458],
        center_of_mass: [0.4981, 0.6020],
        eye: None,
        focus: [0.4981, 0.2240, 0.1800],
        halo_arc: [[0.1480, 0.1730], [0.4951, 0.0082], [0.8487, 0.1730]],
    },
    CutoutMetrics {
        name: "hourglass",
        id: "cutout.hourglass",
        path: "library/classical_greyscale/hourglass.png",
        dims: (512, 852),
        alpha_bbox: [0.1230, 0.0540, 0.8750, 0.9448],
        center_of_mass: [0.4983, 0.5156],
        eye: None,
        focus: [0.4983, 0.4600, 0.1600],
        halo_arc: [[0.0200, 0.0900], [0.4180, 0.0040], [0.9800, 0.0900]],
    },
    CutoutMetrics {
        name: "ancient_coin",
        id: "cutout.ancient_coin",
        path: "library/classical_greyscale/ancient_coin.png",
        dims: (706, 748),
        alpha_bbox: [0.0567, 0.0535, 0.9419, 0.9452],
        center_of_mass: [0.4996, 0.4977],
        eye: None,
        focus: [0.4996, 0.4977, 0.3000],
        halo_arc: [[0.0227, 0.2360], [0.4603, 0.0107], [0.9773, 0.2360]],
    },
    CutoutMetrics {
        name: "amphora",
        id: "cutout.amphora",
        path: "library/classical_greyscale/amphora.png",
        dims: (722, 1216),
        alpha_bbox: [0.0900, 0.0543, 0.9086, 0.9457],
        center_of_mass: [0.4994, 0.4765],
        eye: None,
        focus: [0.4994, 0.4500, 0.2200],
        halo_arc: [[0.0100, 0.1200], [0.4072, 0.0033], [0.9900, 0.1200]],
    },
    CutoutMetrics {
        name: "column",
        id: "cutout.column",
        path: "library/classical_greyscale/column.png",
        dims: (537, 1774),
        alpha_bbox: [0.1769, 0.0536, 0.8212, 0.9459],
        center_of_mass: [0.4973, 0.5163],
        eye: None,
        focus: [0.4973, 0.1000, 0.1600],
        halo_arc: [[0.0800, 0.0550], [0.2384, 0.0150], [0.9100, 0.0550]],
    },
    CutoutMetrics {
        name: "broken_column",
        id: "cutout.broken_column",
        path: "library/classical_greyscale/broken_column.png",
        dims: (841, 661),
        alpha_bbox: [0.0535, 0.0681, 0.9453, 0.9304],
        center_of_mass: [0.4903, 0.4890],
        eye: None,
        focus: [0.4903, 0.4890, 0.2800],
        halo_arc: [[0.0100, 0.2500], [0.2212, 0.0060], [0.9800, 0.2500]],
    },
    CutoutMetrics {
        name: "clenched_fist",
        id: "cutout.clenched_fist",
        path: "library/classical_greyscale/clenched_fist.png",
        dims: (532, 894),
        alpha_bbox: [0.0921, 0.0537, 0.9079, 0.9452],
        center_of_mass: [0.5108, 0.4827],
        eye: None,
        focus: [0.5108, 0.3200, 0.2200],
        halo_arc: [[0.0320, 0.1499], [0.4718, 0.0089], [0.9793, 0.1499]],
    },
    CutoutMetrics {
        name: "greek_helmet",
        id: "cutout.greek_helmet",
        path: "library/classical_greyscale/greek_helmet.png",
        dims: (552, 889),
        alpha_bbox: [0.0870, 0.0540, 0.9112, 0.9449],
        center_of_mass: [0.4993, 0.4694],
        eye: Some([0.3533, 0.4859]),
        focus: [0.4993, 0.4694, 0.2200],
        halo_arc: [[0.0181, 0.1584], [0.4928, 0.0090], [0.9806, 0.1584]],
    },
    // classical_concepts
    CutoutMetrics {
        name: "thinker_statue",
        id: "cutout.thinker_statue",
        path: "library/classical_concepts/thinker_statue.png",
        dims: (742, 1189),
        alpha_bbox: [0.0863, 0.0538, 0.9124, 0.9453],
        center_of_mass: [0.4751, 0.5648],
        eye: Some([0.6536, 0.1556]),
        focus: [0.6500, 0.2400, 0.2000],
        halo_arc: [[0.0721, 0.1270], [0.5620, 0.0172], [0.7756, 0.1270]],
    },
    CutoutMetrics {
        name: "chess_king",
        id: "cutout.chess_king",
        path: "library/classical_concepts/chess_king.png",
        dims: (514, 1220),
        alpha_bbox: [0.1265, 0.0533, 0.8716, 0.9459],
        center_of_mass: [0.4971, 0.5697],
        eye: None,
        focus: [0.4971, 0.1200, 0.1400],
        halo_arc: [[0.2066, 0.0939], [0.4767, 0.0330], [0.7856, 0.0939]],
    },
    CutoutMetrics {
        name: "chess_knight",
        id: "cutout.chess_knight",
        path: "library/classical_concepts/chess_knight.png",
        dims: (650, 1210),
        alpha_bbox: [0.1000, 0.0537, 0.8985, 0.9455],
        center_of_mass: [0.5327, 0.5135],
        eye: Some([0.3800, 0.2438]),
        focus: [0.3800, 0.2438, 0.2000],
        halo_arc: [[0.0926, 0.1311], [0.4138, 0.0150], [0.9566, 0.1311]],
    },
    CutoutMetrics {
        name: "marble_masks",
        id: "cutout.marble_masks",
        path: "library/classical_concepts/marble_masks.png",
        dims: (890, 991),
        alpha_bbox: [0.0596, 0.0535, 0.9393, 0.9455],
        center_of_mass: [0.4958, 0.4614],
        eye: Some([0.3685, 0.3108]),
        focus: [0.4958, 0.4614, 0.2600],
        halo_arc: [[0.0112, 0.2230], [0.4337, 0.0081], [0.9685, 0.2230]],
    },
    CutoutMetrics {
        name: "owl_statue",
        id: "cutout.owl_statue",
        path: "library/classical_concepts/owl_statue.png",
        dims: (542, 1199),
        alpha_bbox: [0.1181, 0.0534, 0.8801, 0.9458],
        center_of_mass: [0.5052, 0.5154],
        eye: Some([0.4004, 0.1443]),
        focus: [0.5052, 0.1600, 0.1800],
        halo_arc: [[0.0200, 0.0850], [0.2417, 0.0033], [0.9300, 0.0850]],
    },
    CutoutMetrics {
        name: "armillary_sphere",
        id: "cutout.armillary_sphere",
        path: "library/classical_concepts/armillary_sphere.png",
        dims: (791, 1302),
        alpha_bbox: [0.0885, 0.0538, 0.9102, 0.9455],
        center_of_mass: [0.5003, 0.4637],
        eye: None,
        focus: [0.5003, 0.3500, 0.1800],
        halo_arc: [[0.0695, 0.1461], [0.4905, 0.0154], [0.9305, 0.1461]],
    },
    CutoutMetrics {
        name: "marble_brain",
        id: "cutout.marble_brain",
        path: "library/classical_concepts/marble_brain.png",
        dims: (782, 1087),
        alpha_bbox: [0.0754, 0.0534, 0.9246, 0.9457],
        center_of_mass: [0.5160, 0.4691],
        eye: None,
        focus: [0.5000, 0.2843, 0.2800],
        halo_arc: [[0.0128, 0.1819], [0.4783, 0.0074], [0.9834, 0.1819]],
    },
    CutoutMetrics {
        name: "winged_victory",
        id: "cutout.winged_victory",
        path: "library/classical_concepts/winged_victory.png",
        dims: (809, 1323),
        alpha_bbox: [0.0878, 0.0537, 0.9110, 0.9456],
        center_of_mass: [0.4898, 0.4755],
        eye: None,
        focus: [0.4898, 0.2721, 0.2200],
        halo_arc: [[0.0500, 0.0600], [0.4900, 0.0150], [0.9500, 0.1200]],
    },
    CutoutMetrics {
        name: "atlas_globe",
        id: "cutout.atlas_globe",
        path: "library/classical_concepts/atlas_globe.png",
        dims: (673, 1276),
        alpha_bbox: [0.1010, 0.0533, 0.8975, 0.9459],
        center_of_mass: [0.5116, 0.4819],
        eye: None,
        focus: [0.5022, 0.2750, 0.2800],
        halo_arc: [[0.0740, 0.1288], [0.5022, 0.0155], [0.9334, 0.1288]],
    },
    CutoutMetrics {
        name: "lion_statue",
        id: "cutout.lion_statue",
        path: "library/classical_concepts/lion_statue.png",
        dims: (949, 985),
        alpha_bbox: [0.0558, 0.0538, 0.9431, 0.9452],
        center_of_mass: [0.5048, 0.5963],
        eye: Some([0.5859, 0.1888]),
        focus: [0.6000, 0.2500, 0.2400],
        halo_arc: [[0.3560, 0.1253], [0.6217, 0.0181], [0.8011, 0.1253]],
    },
    // aikakirja_ornaments
    CutoutMetrics {
        name: "ornament_00cd",
        id: "cutout.ornament_00cd",
        path: "library/aikakirja_ornaments/ornament_00cd.png",
        dims: (1146, 1120),
        alpha_bbox: [0.0532, 0.0545, 0.9459, 0.9446],
        center_of_mass: [0.4996, 0.5068],
        eye: None,
        focus: [0.4996, 0.5068, 0.1500],
        halo_arc: [[0.0400, 0.2200], [0.5934, 0.0080], [0.9500, 0.2200]],
    },
    CutoutMetrics {
        name: "ornament_00ee",
        id: "cutout.ornament_00ee",
        path: "library/aikakirja_ornaments/ornament_00ee.png",
        dims: (1146, 1060),
        alpha_bbox: [0.0532, 0.0575, 0.9459, 0.9415],
        center_of_mass: [0.4319, 0.5345],
        eye: None,
        focus: [0.4319, 0.5345, 0.2000],
        halo_arc: [[0.0768, 0.1917], [0.2653, 0.0104], [0.7478, 0.1917]],
    },
    CutoutMetrics {
        name: "ornament_00f4",
        id: "cutout.ornament_00f4",
        path: "library/aikakirja_ornaments/ornament_00f4.png",
        dims: (1146, 1035),
        alpha_bbox: [0.0532, 0.0589, 0.9459, 0.9401],
        center_of_mass: [0.5026, 0.5249],
        eye: None,
        focus: [0.5026, 0.5249, 0.2000],
        halo_arc: [[0.0300, 0.2000], [0.5314, 0.0039], [0.9900, 0.2000]],
    },
    // chromatic industrial cutouts
    CutoutMetrics {
        name: "argon_core",
        id: "cutout.argon_core",
        path: "generated/argon_core.png",
        dims: (1024, 1024),
        alpha_bbox: [0.1660, 0.1445, 0.6504, 0.7315],
        center_of_mass: [0.4948, 0.5305],
        eye: None,
        focus: [0.4948, 0.4500, 0.2400],
        halo_arc: [[0.1400, 0.2200], [0.4948, 0.0800], [0.8500, 0.2200]],
    },
    CutoutMetrics {
        name: "krona_euv",
        id: "cutout.krona_euv",
        path: "generated/krona_euv.png",
        dims: (1024, 1024),
        alpha_bbox: [0.1543, 0.0605, 0.6865, 0.8858],
        center_of_mass: [0.4686, 0.4938],
        eye: None,
        focus: [0.4800, 0.4800, 0.2200],
        halo_arc: [[0.1400, 0.1400], [0.4800, 0.0250], [0.8500, 0.1400]],
    },
    CutoutMetrics {
        name: "halden_6g",
        id: "cutout.halden_6g",
        path: "generated/halden_6g.png",
        dims: (1024, 1024),
        alpha_bbox: [0.1631, 0.1572, 0.6758, 0.6660],
        center_of_mass: [0.5025, 0.4931],
        eye: None,
        focus: [0.4600, 0.4700, 0.2200],
        halo_arc: [[0.1500, 0.2200], [0.5000, 0.0900], [0.8500, 0.2200]],
    },
    CutoutMetrics {
        name: "india_grid",
        id: "cutout.india_grid",
        path: "generated/india_grid.png",
        dims: (1024, 1024),
        alpha_bbox: [0.1484, 0.0732, 0.6807, 0.8663],
        center_of_mass: [0.4910, 0.5050],
        eye: None,
        focus: [0.4910, 0.3600, 0.2400],
        halo_arc: [[0.1400, 0.1600], [0.5000, 0.0300], [0.8400, 0.1600]],
    },
    CutoutMetrics {
        name: "guardrail_lock",
        id: "cutout.guardrail_lock",
        path: "generated/guardrail_lock.png",
        dims: (1024, 1024),
        alpha_bbox: [0.0410, 0.0537, 0.9180, 0.8906],
        center_of_mass: [0.5018, 0.4942],
        eye: Some([0.5000, 0.5000]),
        focus: [0.5000, 0.5000, 0.2600],
        halo_arc: [[0.0600, 0.1800], [0.5000, 0.0200], [0.9400, 0.1800]],
    },
];

pub fn cutout_metrics(name: &str) -> Option<CutoutMetrics> {
    BUNDLED_CUTOUT_METRICS
        .iter()
        .copied()
        .find(|m| m.name == name)
}

/// Bundled cutout lookup: `(asset_id, relative_path_under_assets, (width, height))`.
pub fn resolve_cutout(name: &str) -> Option<(&'static str, &'static str, (u32, u32))> {
    cutout_metrics(name).map(|m| (m.id, m.path, m.dims))
}

const DEFAULT_GRID_ASSETS: [&str; 9] = [
    "armillary_sphere",
    "laurel",
    "ancient_coin",
    "scales",
    "ornament_00cd",
    "hourglass",
    "owl_statue",
    "greek_helmet",
    "atlas_globe",
];

impl PhotoStack {
    pub fn check(&self, length: f64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if self.steps.is_empty() || self.steps.len() > 32 {
            out.push((
                ".steps".to_string(),
                format!("needs 1..=32 steps, got {}", self.steps.len()),
            ));
        }
        for (i, s) in self.steps.iter().enumerate() {
            let sp = format!(".steps[{i}]");
            if s.beat.is_some() {
                out.push((
                    format!("{sp}.beat"),
                    "beat not in the music analysis (or no music)".to_string(),
                ));
            }
            if !(s.at.is_finite() && s.at >= 0.0 && s.at < length - 1e-4) {
                out.push((
                    format!("{sp}.at"),
                    format!("must be in [0, {length:.2}), got {}", s.at),
                ));
            }
            if let Some(a) = &s.asset {
                if resolve_cutout(a).is_none() {
                    out.push((
                        format!("{sp}.asset"),
                        format!("unknown bundled cutout '{a}'"),
                    ));
                }
            }
            for (k, a) in s.assets.iter().enumerate() {
                if resolve_cutout(a).is_none() {
                    out.push((
                        format!("{sp}.assets[{k}]"),
                        format!("unknown bundled cutout '{a}'"),
                    ));
                }
            }
            for (field, val) in [
                ("text", &s.text),
                ("to_text", &s.to_text),
                ("subtext", &s.subtext),
            ] {
                if let Some(t) = val {
                    if t.trim().is_empty() || t.chars().count() > 24 || t.contains('\n') {
                        out.push((format!("{sp}.{field}"), "1..=24 characters".to_string()));
                    } else if let Some(g) = crate::device::banned_glyph(t) {
                        out.push((
                            format!("{sp}.{field}"),
                            format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                        ));
                    }
                }
            }
        }
        out
    }
}

/// Collects all image assets referenced by `PhotoStack` components in `p`.
pub(crate) fn assets(p: &MotionProgram) -> Vec<Asset> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut add = |name: &str| {
        if let Some((id, path, _)) = resolve_cutout(name) {
            if seen.insert(id) {
                out.push(Asset {
                    id: id.to_string(),
                    kind: AssetKind::Image,
                    path: path.to_string(),
                    sprite: None,
                });
            }
        }
    };
    for c in &p.components {
        let ComponentBody::PhotoStack(ps) = &c.body else {
            continue;
        };
        for s in &ps.steps {
            if let Some(a) = &s.asset {
                add(a);
            }
            for a in &s.assets {
                add(a);
            }
            match s.layout {
                PhotoLayout::Grid if s.assets.is_empty() => {
                    for a in DEFAULT_GRID_ASSETS {
                        add(a);
                    }
                }
                PhotoLayout::WordMorph if s.assets.is_empty() => {
                    add("armillary_sphere");
                    add("atlas_globe");
                }
                PhotoLayout::Hero if s.asset.is_none() => {
                    add("marble_bust");
                }
                PhotoLayout::BrandMark if s.stroke == Some(AccentStroke::RosetteCircle) => {
                    add(s.asset.as_deref().unwrap_or("ornament_00cd"));
                }
                _ => {}
            }
        }
    }
    out
}

fn engraving_treatment(seed: u64) -> ImageTreatment {
    ImageTreatment {
        preset: TreatmentPreset::EditorialMonochrome,
        desaturate: 1.0,
        brightness: 0.02,
        contrast: 1.24,
        duotone: None,
        tint: None,
        grain: 0.02,
        seed,
        edge: None,
        shadow: None,
        sticker: None,
    }
}

/// Builds an 8-facet architectural compass / octagonal emblem SVG path in a `size × size` box.
fn starburst_path(size: f32, _spokes: usize, _inner_ratio: f32) -> String {
    let c = size / 2.0;
    let r_cardinal = size * 0.46;
    let r_diagonal = size * 0.34;
    let mut d = String::new();
    for i in 0..8 {
        let ang = (i as f32 / 8.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { r_cardinal } else { r_diagonal };
        let px = c + r * ang.cos();
        let py = c + r * ang.sin();
        if i == 0 {
            d.push_str(&format!("M{px:.1},{py:.1}"));
        } else {
            d.push_str(&format!(" L{px:.1},{py:.1}"));
        }
    }
    d.push_str(" Z");
    d
}

/// Circle path in a `w × h` box for `Trim` animation.
fn ellipse_path(w: f32, h: f32) -> String {
    let (rx, ry) = (w * 0.46, h * 0.46);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let k = 0.552_284_8;
    let (kx, ky) = (rx * k, ry * k);
    format!(
        "M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} Z",
        cx,
        cy - ry,
        cx + kx,
        cy - ry,
        cx + rx,
        cy - ky,
        cx + rx,
        cy,
        cx + rx,
        cy + ky,
        cx + kx,
        cy + ry,
        cx,
        cy + ry,
        cx - kx,
        cy + ry,
        cx - rx,
        cy + ky,
        cx - rx,
        cy,
        cx - rx,
        cy - ky,
        cx - kx,
        cy - ry,
        cx,
        cy - ry
    )
}

/// Horizontal manuscript ledger lines placed outside `avoid` ranges so
/// type and cutouts never sit on a line (`busy_under_type` == 0).
#[allow(clippy::too_many_arguments)]
fn push_ruled_lines(
    layers: &mut Vec<Layer>,
    track: &mut Track,
    cx: &Ctx,
    prefix: &str,
    candidate_ys: &[f32],
    avoid: &[(f32, f32)],
    col: Color,
    z: i32,
    tempo: f64,
) {
    let (w, h) = (cx.w, cx.h);
    let rw = (w * 0.78).round();
    let sw = (cx.short / 540.0).round().max(2.0);
    let mut line_idx = 0;
    for &ry in candidate_ys {
        if ry <= 0.0 || ry >= h {
            continue;
        }
        if avoid
            .iter()
            .any(|&(y0, y1)| ry >= y0 - 18.0 && ry <= y1 + 18.0)
        {
            continue;
        }
        let lid = format!("{prefix}.rule{line_idx}");
        line_idx += 1;
        let d = format!("M0,{sw:.1} L{rw:.1},{sw:.1}");
        layers.push(layer(
            &lid,
            LayerKind::Path {
                d,
                fill: None,
                stroke: Some(Stroke {
                    color: col,
                    width: sw,
                    brush: None,
                }),
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [w / 2.0, ry],
            [rw, sw * 2.0],
            z,
        ));
        track.push(
            &lid,
            line_idx as f64 * 0.02,
            0.22 * tempo,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Trim { from: 0.0, to: 1.0 },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_cutout(
    layers: &mut Vec<Layer>,
    track: &mut Track,
    cx: &Ctx,
    lid: &str,
    asset_name: &str,
    pos: [f32; 2],
    box_h: f32,
    z: i32,
    at: f64,
    dur: f64,
    seed: u64,
) -> [f32; 4] {
    let Some((aid, rel_path, (iw, ih))) = resolve_cutout(asset_name) else {
        return [pos[0] - box_h / 2.0, pos[1] - box_h / 2.0, box_h, box_h];
    };
    let aspect = iw as f32 / (ih.max(1) as f32);
    let mut bh = box_h.round();
    let mut bw = (bh * aspect).round();
    let max_w = (cx.w * 0.86).round();
    if bw > max_w {
        bw = max_w;
        bh = (bw / aspect).round();
    }
    let x0 = pos[0] - bw / 2.0;
    let y0 = pos[1] - bh / 2.0;
    let treatment = if rel_path.starts_with("generated/") {
        None
    } else {
        Some(engraving_treatment(seed))
    };
    layers.push(layer(
        lid,
        LayerKind::Image {
            asset: aid.to_string(),
            fit: Fit::Contain,
            treatment,
            playback: None,
            insert: None,
        },
        pos,
        [bw, bh],
        z,
    ));
    let pop_dur = (dur * 0.45).clamp(0.04, 0.24);
    track.push(
        lid,
        at,
        pop_dur,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.88,
            to: 1.0,
            axis: Default::default(),
        },
    );
    let rem = dur - pop_dur;
    if rem >= 0.04 {
        track.push(
            lid,
            at + pop_dur,
            rem,
            Shape::Ease(Easing::OutCubic),
            MotionOp::Scale {
                from: 1.0,
                to: 1.06,
                axis: Default::default(),
            },
        );
    }
    [x0, y0, bw, bh]
}

#[allow(clippy::too_many_arguments)]
fn lower_brand_mark(
    cx: &Ctx,
    comp_id: &str,
    sid: &str,
    step: &PhotoStep,
    t0: f64,
    len: f64,
    z: i32,
    idx: usize,
) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let tempo = cx.tempo.factor();
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    if step.stroke == Some(AccentStroke::RosetteCircle) {
        let asset_name = step.asset.as_deref().unwrap_or("ornament_00cd");
        let c_lid = format!("{sid}.rosette");
        push_cutout(
            &mut layers,
            &mut track,
            cx,
            &c_lid,
            asset_name,
            [cx.w / 2.0, cx.h / 2.0],
            cx.short * 0.48,
            z + 1,
            0.0,
            len,
            idx as u64 + 1,
        );
        let ring_d = (cx.short * 0.66).round();
        let sw = (cx.short * 0.008).round().max(5.0);
        let r_lid = format!("{sid}.ring");
        layers.push(layer(
            &r_lid,
            LayerKind::Path {
                d: ellipse_path(ring_d, ring_d),
                fill: None,
                stroke: Some(Stroke {
                    color: ink,
                    width: sw,
                    brush: None,
                }),
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [cx.w / 2.0, cx.h / 2.0],
            [ring_d, ring_d],
            z + 2,
        ));
        track.push(
            &r_lid,
            0.0,
            (len * 0.75).max(0.2).min(len),
            Shape::Ease(Easing::OutQuint),
            MotionOp::Trim { from: 0.0, to: 1.0 },
        );
        let dot_sz = (cx.short * 0.032).round();
        let d_lid = format!("{sid}.tip");
        layers.push(layer(
            &d_lid,
            LayerKind::Path {
                d: ellipse_path(dot_sz, dot_sz),
                fill: Some(motion_core::scene::Fill::Solid(accent)),
                stroke: None,
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [cx.w / 2.0 + ring_d * 0.33, cx.h / 2.0 - ring_d * 0.31],
            [dot_sz, dot_sz],
            z + 3,
        ));
        track.push(
            &d_lid,
            0.0,
            (0.24 * tempo).min(len),
            Shape::Spring(Spring::Pop),
            MotionOp::Scale {
                from: 0.4,
                to: 1.0,
                axis: Default::default(),
            },
        );
        return scene(sid, t0, len, layers, track.motions);
    }

    let word = step.text.as_deref().unwrap_or("Studio");
    let target_span = cx.short * 0.66;
    let fs = cx.fit(
        word,
        FontRole::SerifEmotional,
        -0.01,
        false,
        target_span * 0.72,
        (cx.short * 0.15).round(),
    );
    let tw = cx.line_width(word, FontRole::SerifEmotional, fs, -0.01, false);
    let star_sz = (target_span - tw)
        .clamp(cx.short * 0.14, cx.short * 0.24)
        .round();
    let gap = (fs * 0.28).round().max(14.0);
    let total_w = star_sz + gap + tw;
    let x0 = (cx.w - total_w) / 2.0;
    let cy = cx.h / 2.0;

    let s_lid = format!("{sid}.star");
    layers.push(layer(
        &s_lid,
        LayerKind::Path {
            d: starburst_path(star_sz, 12, 0.24),
            fill: Some(motion_core::scene::Fill::Solid(accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [x0 + star_sz / 2.0, cy],
        [star_sz, star_sz],
        z + 2,
    ));
    track.push(
        &s_lid,
        0.0,
        (0.28 * tempo).min(len),
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.65,
            to: 1.0,
            axis: Default::default(),
        },
    );
    track.push(
        &s_lid,
        0.0,
        len,
        Shape::Ease(Easing::OutCubic),
        MotionOp::Rotate {
            from: -18.0,
            to: 0.0,
        },
    );

    let t_lid = format!("{sid}.word");
    let t_col = cx.paint(comp_id, Swatch::Ink, Use::Text(fs));
    layers.push(cx.text(
        &t_lid,
        word,
        FontRole::SerifEmotional,
        fs,
        t_col,
        -0.01,
        false,
        [x0 + star_sz + gap + tw / 2.0, cy],
        z + 2,
    ));
    track.push(
        &t_lid,
        0.0,
        (0.26 * tempo).min(len),
        Shape::Spring(Spring::Snap),
        MotionOp::Scale {
            from: 0.92,
            to: 1.0,
            axis: Default::default(),
        },
    );
    scene(sid, t0, len, layers, track.motions)
}

#[allow(clippy::too_many_arguments)]
fn lower_word_phase(
    cx: &Ctx,
    comp_id: &str,
    sid: &str,
    word: &str,
    subtext: Option<&str>,
    cutouts: &[String],
    stroke_kind: AccentStroke,
    ruled: bool,
    second_phase: bool,
    t0: f64,
    len: f64,
    z: i32,
    seed: u64,
) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let tempo = cx.tempo.factor();
    let roles = cx.roles(comp_id);
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let fs = cx.fit(
        word,
        FontRole::SerifEmotional,
        -0.01,
        false,
        cx.w * 0.68,
        (cx.short * 0.21).round(),
    );
    let tw = cx.line_width(word, FontRole::SerifEmotional, fs, -0.01, false);
    let th = cx.ink_height(word, FontRole::SerifEmotional, fs, -0.01, false);
    let wy = if subtext.is_some() {
        cx.h * 0.45
    } else {
        cx.h * 0.49
    };

    let mut avoid_y1 = wy + th * 0.65;
    if let Some(sub) = subtext {
        let sfs = cx.fit(
            sub,
            FontRole::SerifEmotional,
            -0.01,
            false,
            cx.w * 0.62,
            (cx.short * 0.105).round(),
        );
        let sth = cx.ink_height(sub, FontRole::SerifEmotional, sfs, -0.01, false);
        let sy = wy + th * 0.62 + sth * 0.72;
        avoid_y1 = sy + sth * 0.65;
        let s_col = cx.paint(comp_id, Swatch::Ink, Use::Text(sfs));
        let s_lid = format!("{sid}.sub");
        layers.push(cx.text(
            &s_lid,
            sub,
            FontRole::SerifEmotional,
            sfs,
            s_col,
            -0.01,
            false,
            [cx.w / 2.0, sy],
            z + 3,
        ));
        track.push(
            &s_lid,
            0.0,
            (0.24 * tempo).min(len),
            Shape::Spring(Spring::Snap),
            MotionOp::Scale {
                from: 0.9,
                to: 1.0,
                axis: Default::default(),
            },
        );
    }

    // Cutouts: at most 1 purposeful cutout per phase, zero duplicate stamps!
    // Phase A (!second_phase): centered above the word at [cx.w / 2.0, cx.h * 0.24] with box_h = cx.h * 0.23
    // Phase B (second_phase): centered below the word at [cx.w / 2.0, cx.h * 0.77] with box_h = cx.h * 0.23 (or above at 0.24 * h if subtext is some)
    let mut cutout_box: Option<[f32; 4]> = None;
    if !second_phase {
        if let Some(top_asset) = cutouts.first() {
            let cid = format!("{sid}.cut0");
            let rect = push_cutout(
                &mut layers,
                &mut track,
                cx,
                &cid,
                top_asset,
                [cx.w / 2.0, cx.h * 0.21],
                cx.h * 0.28,
                z + 2,
                0.0,
                len,
                seed,
            );
            cutout_box = Some(rect);
        }
    } else if let Some(bot_asset) = cutouts.get(1).or_else(|| cutouts.first()) {
        let cid = format!("{sid}.cut1");
        let cy = if subtext.is_some() {
            cx.h * 0.20
        } else {
            cx.h * 0.78
        };
        let rect = push_cutout(
            &mut layers,
            &mut track,
            cx,
            &cid,
            bot_asset,
            [cx.w / 2.0, cy],
            cx.h * 0.28,
            z + 2,
            0.0,
            len,
            seed + 7,
        );
        cutout_box = Some(rect);
    }

    let has_underline = second_phase
        && matches!(
            stroke_kind,
            AccentStroke::TopSlash | AccentStroke::Flourish | AccentStroke::StrikeMorph
        );

    let mut avoid = Vec::new();
    avoid.push((wy - th * 0.65, avoid_y1));
    if has_underline {
        avoid.push((wy + th * 0.58, wy + th * 0.78));
    }
    if let Some(cr) = cutout_box {
        avoid.push((cr[1], cr[1] + cr[3]));
    }

    let mut candidate_ys = vec![cx.h * 0.16, cx.h * 0.84];
    if ruled {
        candidate_ys.push(wy - th * 0.72);
        if !has_underline && subtext.is_none() {
            candidate_ys.push(wy + th * 0.72);
        }
    }

    push_ruled_lines(
        &mut layers,
        &mut track,
        cx,
        sid,
        &candidate_ys,
        &avoid,
        roles.structure_strong,
        z + 1,
        tempo,
    );

    let t_col = cx.paint(comp_id, Swatch::Ink, Use::Text(fs));
    let w_lid = format!("{sid}.word");
    layers.push(cx.text(
        &w_lid,
        word,
        FontRole::SerifEmotional,
        fs,
        t_col,
        -0.01,
        false,
        [cx.w / 2.0, wy],
        z + 3,
    ));
    track.push(
        &w_lid,
        0.0,
        0.24 * tempo,
        Shape::Spring(Spring::Snap),
        MotionOp::Scale {
            from: if second_phase { 1.08 } else { 0.92 },
            to: 1.0,
            axis: Default::default(),
        },
    );

    // Hand-painted editorial stroke anchored to the word's glyph box (tw, th, wy).
    let sw = (cx.short * 0.009).round().max(5.0);
    let st_lid = format!("{sid}.stroke");
    match stroke_kind {
        AccentStroke::TopSlash | AccentStroke::Flourish => {
            if second_phase {
                // Phase B: clean sweeping underline directly beneath the morphed word's baseline
                let bw = (tw * 0.96).max(80.0);
                let bh = (cx.short * 0.024).round().max(14.0);
                let d = format!(
                    "M0,{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
                    bh * 0.7,
                    bw * 0.33,
                    bh * 0.25,
                    bw * 0.67,
                    bh * 0.75,
                    bw,
                    bh * 0.35
                );
                layers.push(layer(
                    &st_lid,
                    LayerKind::Path {
                        d,
                        fill: None,
                        stroke: Some(Stroke {
                            color: accent,
                            width: sw,
                            brush: stroke_kind.brush_spec(11),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    [cx.w / 2.0, wy + th * 0.68],
                    [bw, bh],
                    z + 4,
                ));
            } else {
                // Phase A: terracotta editorial proofreader mark just above the leading letter
                let bw = (cx.short * 0.07).round().max(28.0);
                let bh = (cx.short * 0.045).round().max(18.0);
                let d = format!("M0,{:.1} L{:.1},0", bh * 0.85, bw);
                layers.push(layer(
                    &st_lid,
                    LayerKind::Path {
                        d,
                        fill: None,
                        stroke: Some(Stroke {
                            color: accent,
                            width: sw,
                            brush: stroke_kind.brush_spec(12),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    [cx.w / 2.0 - tw * 0.35, wy - th * 0.78],
                    [bw, bh],
                    z + 4,
                ));
            }
            track.push(
                &st_lid,
                0.0,
                (len * 0.65).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
        AccentStroke::StrikeMorph => {
            if second_phase {
                // Phase B: full baseline underline beneath the morphed word
                let bw = (tw * 0.96).max(80.0);
                let bh = (cx.short * 0.02).round().max(12.0);
                let d = format!("M0,{:.1} L{:.1},{:.1}", bh * 0.5, bw, bh * 0.5);
                layers.push(layer(
                    &st_lid,
                    LayerKind::Path {
                        d,
                        fill: None,
                        stroke: Some(Stroke {
                            color: accent,
                            width: sw,
                            brush: stroke_kind.brush_spec(13),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    [cx.w / 2.0, wy + th * 0.68],
                    [bw, bh],
                    z + 4,
                ));
            } else {
                // Phase A: editorial slash/caret at the leading letter edge
                let bw = (cx.short * 0.06).round().max(24.0);
                let bh = (th * 0.65).clamp(16.0, 44.0).round();
                let d = format!("M0,{:.1} L{:.1},0", bh, bw);
                layers.push(layer(
                    &st_lid,
                    LayerKind::Path {
                        d,
                        fill: None,
                        stroke: Some(Stroke {
                            color: accent,
                            width: sw,
                            brush: stroke_kind.brush_spec(14),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    [cx.w / 2.0 - tw * 0.38, wy - th * 0.1],
                    [bw, bh],
                    z + 4,
                ));
            }
            track.push(
                &st_lid,
                0.0,
                (len * 0.65).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
        _ => {
            // Terracotta dot on the baseline immediately to the right of the word
            let dot_sz = (cx.short * 0.034).round().max(22.0);
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d: ellipse_path(dot_sz, dot_sz),
                    fill: Some(motion_core::scene::Fill::Solid(accent)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                [cx.w / 2.0 + tw * 0.5 + dot_sz * 0.85, wy + th * 0.32],
                [dot_sz, dot_sz],
                z + 4,
            ));
            track.push(
                &st_lid,
                0.0,
                0.22 * tempo,
                Shape::Spring(Spring::Pop),
                MotionOp::Scale {
                    from: 0.3,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
    }

    scene(sid, t0, len, layers, track.motions)
}

#[allow(clippy::too_many_arguments)]
fn lower_grid(
    cx: &Ctx,
    _comp_id: &str,
    sid: &str,
    step: &PhotoStep,
    t0: f64,
    len: f64,
    z: i32,
    seed: u64,
) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let tempo = cx.tempo.factor();
    let accent = cx.paint(_comp_id, Swatch::Accent, Use::Line { meaning: true });

    let grid_span = cx.short * 0.78;
    let cell = grid_span / 3.0;
    let x0 = cx.w / 2.0 - grid_span / 2.0 + cell / 2.0;
    let y0 = cx.h / 2.0 - grid_span / 2.0 + cell / 2.0;

    for (idx, &default_asset) in DEFAULT_GRID_ASSETS.iter().enumerate() {
        let row = idx / 3;
        let col = idx % 3;
        let cx_i = x0 + col as f32 * cell;
        let cy_i = y0 + row as f32 * cell;
        let name = step
            .assets
            .get(idx)
            .map(String::as_str)
            .unwrap_or(default_asset);
        let lid = format!("{sid}.g{idx}");
        let delay = (idx as f64 * 0.018).min(len * 0.35);
        push_cutout(
            &mut layers,
            &mut track,
            cx,
            &lid,
            name,
            [cx_i, cy_i],
            cell * 0.78,
            z + 1 + idx as i32,
            delay,
            (len - delay).max(0.08),
            seed + idx as u64,
        );
    }

    // Centre accent dot on cell 4.
    let dot_sz = (cx.short * 0.032).round();
    let dlid = format!("{sid}.cdot");
    layers.push(layer(
        &dlid,
        LayerKind::Path {
            d: ellipse_path(dot_sz, dot_sz),
            fill: Some(motion_core::scene::Fill::Solid(accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [dot_sz, dot_sz],
        z + 12,
    ));
    track.push(
        &dlid,
        0.0,
        0.24 * tempo,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.3,
            to: 1.0,
            axis: Default::default(),
        },
    );
    scene(sid, t0, len, layers, track.motions)
}

#[allow(clippy::too_many_arguments)]
fn lower_zoetrope(cx: &Ctx, comp_id: &str, sid: &str, t0: f64, len: f64, z: i32) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let wheel = (cx.short * 0.72).round();
    let r = wheel * 0.38;
    let c = wheel / 2.0;
    let mut d = String::new();
    let spokes = 12usize;
    for i in 0..spokes {
        let ang = (i as f32 / spokes as f32) * std::f32::consts::TAU;
        let px = c + r * ang.cos();
        let py = c + r * ang.sin();
        let tx = -ang.sin();
        let ty = ang.cos();
        let nx = ang.cos();
        let ny = ang.sin();
        let s = wheel * 0.055;
        // Stylised galloping silhouette diamond/runner along the tangent.
        let p0 = [px + tx * s, py + ty * s];
        let p1 = [px + nx * s * 0.55, py + ny * s * 0.55];
        let p2 = [px - tx * s, py - ty * s];
        let p3 = [px - nx * s * 0.45, py - ny * s * 0.45];
        d.push_str(&format!(
            "M{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1} Z ",
            p0[0], p0[1], p1[0], p1[1], p2[0], p2[1], p3[0], p3[1]
        ));
    }
    let w_lid = format!("{sid}.wheel");
    layers.push(layer(
        &w_lid,
        LayerKind::Path {
            d,
            fill: Some(motion_core::scene::Fill::Solid(ink)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [wheel, wheel],
        z + 2,
    ));
    track.push(
        &w_lid,
        0.0,
        len.max(0.2),
        Shape::Ease(Easing::OutCubic),
        MotionOp::Rotate {
            from: 0.0,
            to: 45.0,
        },
    );
    track.push(
        &w_lid,
        0.0,
        0.24,
        Shape::Spring(Spring::Pop),
        MotionOp::Scale {
            from: 0.86,
            to: 1.0,
            axis: Default::default(),
        },
    );

    let ring_lid = format!("{sid}.ring");
    let sw = (cx.short * 0.006).round().max(4.0);
    layers.push(layer(
        &ring_lid,
        LayerKind::Path {
            d: ellipse_path(wheel, wheel),
            fill: None,
            stroke: Some(Stroke {
                color: ink,
                width: sw,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [wheel, wheel],
        z + 1,
    ));
    track.push(
        &ring_lid,
        0.0,
        (len * 0.7).max(0.18),
        Shape::Ease(Easing::OutQuint),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    let dot_sz = (cx.short * 0.026).round();
    let dlid = format!("{sid}.hub");
    layers.push(layer(
        &dlid,
        LayerKind::Path {
            d: ellipse_path(dot_sz, dot_sz),
            fill: Some(motion_core::scene::Fill::Solid(accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [dot_sz, dot_sz],
        z + 3,
    ));
    scene(sid, t0, len, layers, track.motions)
}

#[allow(clippy::too_many_arguments)]
fn lower_hero(
    cx: &Ctx,
    comp_id: &str,
    sid: &str,
    step: &PhotoStep,
    slot: crate::program::Slot,
    t0: f64,
    len: f64,
    z: i32,
    seed: u64,
) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let tempo = cx.tempo.factor();
    let roles = cx.roles(comp_id);
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let is_split = matches!(
        slot,
        crate::program::Slot::Left | crate::program::Slot::Right
    );
    let slot_cx = if is_split {
        crate::taste::slot_center(slot, cx.aspect)[0]
    } else {
        cx.w / 2.0
    };
    let slot_w = crate::taste::slot_width(slot, cx.aspect);
    let hero_box_h = if is_split {
        (cx.h * 0.56).min(slot_w * 0.88)
    } else {
        cx.h * 0.62
    };

    let asset_name = step.asset.as_deref().unwrap_or("marble_bust");
    let c_lid = format!("{sid}.hero");
    let [x0, y0, bw, bh] = push_cutout(
        &mut layers,
        &mut track,
        cx,
        &c_lid,
        asset_name,
        [slot_cx, cx.h * 0.52],
        hero_box_h,
        z + 2,
        0.0,
        len,
        seed,
    );

    if step.ruled && !is_split {
        let hero_avoid = [(y0, y0 + bh)];
        let candidate_ys = [cx.h * 0.14, cx.h * 0.88];
        push_ruled_lines(
            &mut layers,
            &mut track,
            cx,
            sid,
            &candidate_ys,
            &hero_avoid,
            roles.structure_strong,
            z + 1,
            tempo,
        );
    }

    let m = cutout_metrics(asset_name).unwrap_or(BUNDLED_CUTOUT_METRICS[0]);
    let pt = |uv: [f32; 2]| [x0 + uv[0] * bw, y0 + uv[1] * bh];

    let sw = (cx.short * 0.008).round().max(5.0);
    let st_lid = format!("{sid}.stroke");
    let hero_stroke = step.stroke.unwrap_or(AccentStroke::NeckArc);
    match hero_stroke {
        AccentStroke::EyeOval => {
            let center = pt(m.eye.unwrap_or([m.focus[0], m.focus[1]]));
            let ow = (bw * m.focus[2] * 1.15)
                .clamp(cx.short * 0.12, cx.short * 0.26)
                .round();
            let oh = (ow * 0.68).round();
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d: ellipse_path(ow, oh),
                    fill: None,
                    stroke: Some(Stroke {
                        color: accent,
                        width: sw,
                        brush: hero_stroke.brush_spec(21),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                center,
                [ow, oh],
                z + 4,
            ));
            track.push(
                &st_lid,
                0.0,
                (len * 0.7).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
        AccentStroke::StarburstStamp => {
            let center = pt([m.focus[0], m.focus[1]]);
            let ret_sz = (bw * m.focus[2] * 1.45)
                .clamp(cx.short * 0.18, cx.short * 0.30)
                .round();
            let c_half = ret_sz * 0.5;
            let r_ring = ret_sz * 0.36;
            let r_in = ret_sz * 0.24;
            let r_out = ret_sz * 0.48;
            let k = 0.552_284_8 * r_ring;
            let ret_d = format!(
                "M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} Z \
                 M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1}",
                c_half,
                c_half - r_ring,
                c_half + k,
                c_half - r_ring,
                c_half + r_ring,
                c_half - k,
                c_half + r_ring,
                c_half,
                c_half + r_ring,
                c_half + k,
                c_half + k,
                c_half + r_ring,
                c_half,
                c_half + r_ring,
                c_half - k,
                c_half + r_ring,
                c_half - r_ring,
                c_half + k,
                c_half - r_ring,
                c_half,
                c_half - r_ring,
                c_half - k,
                c_half - k,
                c_half - r_ring,
                c_half,
                c_half - r_ring,
                // Top tick
                c_half,
                c_half - r_out,
                c_half,
                c_half - r_in,
                // Bottom tick
                c_half,
                c_half + r_in,
                c_half,
                c_half + r_out,
                // Left tick
                c_half - r_out,
                c_half,
                c_half - r_in,
                c_half,
                // Right tick
                c_half + r_in,
                c_half,
                c_half + r_out,
                c_half,
            );
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d: ret_d,
                    fill: None,
                    stroke: Some(Stroke {
                        color: accent,
                        width: sw + 1.0,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                center,
                [ret_sz, ret_sz],
                z + 4,
            ));
            track.push(
                &st_lid,
                0.0,
                (len * 0.72).max(0.20),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
            track.push(
                &st_lid,
                0.0,
                len.max(0.2),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Rotate {
                    from: -18.0,
                    to: 0.0,
                },
            );
            let dot_sz = (cx.short * 0.022).round().max(10.0);
            let dlid = format!("{sid}.dot");
            layers.push(layer(
                &dlid,
                LayerKind::Path {
                    d: ellipse_path(dot_sz, dot_sz),
                    fill: Some(motion_core::scene::Fill::Solid(accent)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                center,
                [dot_sz, dot_sz],
                z + 5,
            ));
            track.push(
                &dlid,
                0.0,
                0.24 * tempo,
                Shape::Spring(Spring::Pop),
                MotionOp::Scale {
                    from: 0.2,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
        AccentStroke::BracketDot => {
            let focus_xy = pt([m.focus[0], m.focus[1]]);
            let r = bw * m.focus[2] * 0.65;
            let bw_b = (cx.short * 0.06).round().max(24.0);
            let bh_b = (r * 2.2).clamp(cx.short * 0.16, cx.short * 0.32).round();
            let d = format!("M{bw_b:.1},0 L0,0 L0,{bh_b:.1} L{bw_b:.1},{bh_b:.1}");
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d,
                    fill: None,
                    stroke: Some(Stroke {
                        color: ink,
                        width: sw,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                [focus_xy[0] - r * 1.15, focus_xy[1]],
                [bw_b, bh_b],
                z + 4,
            ));
            track.push(
                &st_lid,
                0.0,
                (len * 0.65).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
            let dot_sz = (cx.short * 0.028).round();
            let dlid = format!("{sid}.dot");
            layers.push(layer(
                &dlid,
                LayerKind::Path {
                    d: ellipse_path(dot_sz, dot_sz),
                    fill: Some(motion_core::scene::Fill::Solid(accent)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                focus_xy,
                [dot_sz, dot_sz],
                z + 5,
            ));
            track.push(
                &dlid,
                0.0,
                0.22 * tempo,
                Shape::Spring(Spring::Pop),
                MotionOp::Scale {
                    from: 0.3,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
        AccentStroke::RibCircle => {
            let ring_center = pt([m.center_of_mass[0], m.focus[1].max(0.36)]);
            let cd = if is_split {
                (bw * 1.02).clamp(slot_w * 0.58, slot_w * 0.86).round()
            } else {
                (bw * 1.04).clamp(cx.short * 0.56, cx.short * 0.76).round()
            };
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d: ellipse_path(cd, cd),
                    fill: None,
                    stroke: Some(Stroke {
                        color: ink,
                        width: sw,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                ring_center,
                [cd, cd],
                z + 1, // BEHIND the cutout (hero cutout is at z + 2)!
            ));
            track.push(
                &st_lid,
                0.0,
                (len * 0.72).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
            let dot_sz = (cx.short * 0.030).round();
            let dlid = format!("{sid}.dot");
            layers.push(layer(
                &dlid,
                LayerKind::Path {
                    d: ellipse_path(dot_sz, dot_sz),
                    fill: Some(motion_core::scene::Fill::Solid(accent)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                [ring_center[0] + cd * 0.325, ring_center[1] - cd * 0.325],
                [dot_sz, dot_sz],
                z + 4,
            ));
            track.push(
                &dlid,
                0.0,
                0.22 * tempo,
                Shape::Spring(Spring::Pop),
                MotionOp::Scale {
                    from: 0.3,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
        _ => {
            let p0 = pt(m.halo_arc[0]);
            let p1 = pt(m.halo_arc[1]);
            let p2 = pt(m.halo_arc[2]);
            let min_x = p0[0].min(p1[0]).min(p2[0]);
            let max_x = p0[0].max(p1[0]).max(p2[0]);
            let min_y = p0[1].min(p1[1]).min(p2[1]);
            let max_y = p0[1].max(p1[1]).max(p2[1]);
            let aw = (max_x - min_x).max(10.0);
            let ah = (max_y - min_y).max(10.0);
            let center = [min_x + aw / 2.0, min_y + ah / 2.0];
            let l0 = [p0[0] - min_x, p0[1] - min_y];
            let l1 = [p1[0] - min_x, p1[1] - min_y];
            let l2 = [p2[0] - min_x, p2[1] - min_y];
            let d = format!(
                "M{:.1},{:.1} Q{:.1},{:.1} {:.1},{:.1}",
                l0[0], l0[1], l1[0], l1[1], l2[0], l2[1]
            );
            layers.push(layer(
                &st_lid,
                LayerKind::Path {
                    d,
                    fill: None,
                    stroke: Some(Stroke {
                        color: ink,
                        width: sw,
                        brush: hero_stroke.brush_spec(22),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                center,
                [aw, ah],
                z + 4,
            ));
            track.push(
                &st_lid,
                0.0,
                (len * 0.7).max(0.18),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
            let dot_sz = (cx.short * 0.028).round();
            let dlid = format!("{sid}.dot");
            layers.push(layer(
                &dlid,
                LayerKind::Path {
                    d: ellipse_path(dot_sz, dot_sz),
                    fill: Some(motion_core::scene::Fill::Solid(accent)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                p1,
                [dot_sz, dot_sz],
                z + 5,
            ));
            track.push(
                &dlid,
                0.0,
                0.22 * tempo,
                Shape::Spring(Spring::Pop),
                MotionOp::Scale {
                    from: 0.3,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
    }

    scene(sid, t0, len, layers, track.motions)
}

fn lower_blueprint(cx: &Ctx, comp_id: &str, sid: &str, t0: f64, len: f64, z: i32) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let roles = cx.roles(comp_id);
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let box_sz = (cx.short * 0.72).round();
    let c = box_sz / 2.0;
    let r = box_sz * 0.46;
    let mut axes = String::new();
    for i in 0..6 {
        let ang = (i as f32 / 6.0) * std::f32::consts::PI;
        let dx = r * ang.cos();
        let dy = r * ang.sin();
        axes.push_str(&format!(
            "M{:.1},{:.1} L{:.1},{:.1} ",
            c - dx,
            c - dy,
            c + dx,
            c + dy
        ));
    }
    let sw = (cx.short * 0.005).round().max(4.0);
    let a_lid = format!("{sid}.axes");
    layers.push(layer(
        &a_lid,
        LayerKind::Path {
            d: axes,
            fill: None,
            stroke: Some(Stroke {
                color: roles.structure_strong,
                width: sw,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [box_sz, box_sz],
        z + 1,
    ));

    let s_lid = format!("{sid}.outline");
    layers.push(layer(
        &s_lid,
        LayerKind::Path {
            d: starburst_path(box_sz * 0.86, 12, 0.24),
            fill: None,
            stroke: Some(Stroke {
                color: ink,
                width: sw + 1.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [box_sz * 0.86, box_sz * 0.86],
        z + 2,
    ));
    track.push(
        &s_lid,
        0.0,
        (len * 0.78).max(0.2),
        Shape::Ease(Easing::OutQuint),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    let dot_sz = (cx.short * 0.028).round();
    let dlid = format!("{sid}.hub");
    layers.push(layer(
        &dlid,
        LayerKind::Path {
            d: ellipse_path(dot_sz, dot_sz),
            fill: Some(motion_core::scene::Fill::Solid(accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [dot_sz, dot_sz],
        z + 3,
    ));
    scene(sid, t0, len, layers, track.motions)
}

fn lower_starburst_3d(cx: &Ctx, comp_id: &str, sid: &str, t0: f64, len: f64, z: i32) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let tempo = cx.tempo.factor();
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let darken = |c: Color, f: f32| -> Color {
        Color::rgb(
            (c.r as f32 * f).round() as u8,
            (c.g as f32 * f).round() as u8,
            (c.b as f32 * f).round() as u8,
        )
    };
    let star_sz = (cx.short * 0.64).round();
    let path_d = starburst_path(star_sz, 12, 0.25);

    // Stacked 2.5D/3D depth extrusion slices from back to front.
    let slices = 6usize;
    for k in 0..slices {
        let frac = k as f32 / (slices - 1) as f32;
        let shade = darken(accent, 0.62 + 0.38 * frac);
        let dx = (1.0 - frac) * (cx.short * 0.032);
        let dy = (1.0 - frac) * (cx.short * 0.026);
        let lid = format!("{sid}.ext{k}");
        let shadows = if k == 0 {
            vec![ContactShadow {
                color: Color {
                    a: 38,
                    ..darken(accent, 0.25)
                },
                offset: [0.0, cx.short * 0.045],
                blur: cx.short * 0.035,
            }]
        } else {
            Vec::new()
        };
        let mut l = layer(
            &lid,
            LayerKind::Path {
                d: path_d.clone(),
                fill: Some(motion_core::scene::Fill::Solid(shade)),
                stroke: None,
                fill_rule: PathFillRule::Nonzero,
                shadows,
            },
            [cx.w / 2.0 + dx, cx.h / 2.0 + dy],
            [star_sz, star_sz],
            z + 1 + k as i32,
        );
        l.scale_x = 0.92;
        layers.push(l);
        track.push(
            &lid,
            0.0,
            0.28 * tempo,
            Shape::Spring(Spring::Pop),
            MotionOp::Scale {
                from: 0.72,
                to: 1.0,
                axis: Default::default(),
            },
        );
        track.push(
            &lid,
            0.0,
            len.max(0.2),
            Shape::Ease(Easing::OutCubic),
            MotionOp::Rotate {
                from: -22.0,
                to: 12.0,
            },
        );
    }

    scene(sid, t0, len, layers, track.motions)
}

fn lower_constellation(cx: &Ctx, comp_id: &str, sid: &str, t0: f64, len: f64, z: i32) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let (bw, bh) = (cx.w * 0.74, cx.h * 0.64);
    let pts = [
        [bw * 0.38, bh * 0.42],
        [bw * 0.24, bh * 0.56],
        [bw * 0.48, bh * 0.62],
        [bw * 0.82, bh * 0.34],
    ];
    let d = format!(
        "M{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1}",
        pts[0][0], pts[0][1], pts[1][0], pts[1][1], pts[2][0], pts[2][1], pts[3][0], pts[3][1]
    );
    let sw = (cx.short * 0.006).round().max(4.0);
    let c_lid = format!("{sid}.lines");
    layers.push(layer(
        &c_lid,
        LayerKind::Path {
            d,
            fill: None,
            stroke: Some(Stroke {
                color: ink,
                width: sw,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [bw, bh],
        z + 2,
    ));
    track.push(
        &c_lid,
        0.0,
        (len * 0.72).max(0.18),
        Shape::Ease(Easing::OutQuint),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    // Star nodes + terracotta pointer at the terminal vertex.
    let ox = cx.w / 2.0 - bw / 2.0;
    let oy = cx.h / 2.0 - bh / 2.0;
    for (k, p) in pts.iter().enumerate() {
        let n_lid = format!("{sid}.node{k}");
        let is_tip = k + 1 == pts.len();
        let sz = if is_tip {
            (cx.short * 0.048).round()
        } else {
            (cx.short * 0.018).round()
        };
        let path = if is_tip {
            format!("M{:.1},0 L{sz:.1},{sz:.1} L0,{sz:.1} Z", sz * 0.5)
        } else {
            ellipse_path(sz, sz)
        };
        layers.push(layer(
            &n_lid,
            LayerKind::Path {
                d: path,
                fill: Some(motion_core::scene::Fill::Solid(if is_tip {
                    accent
                } else {
                    ink
                })),
                stroke: None,
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [ox + p[0], oy + p[1]],
            [sz, sz],
            z + 3,
        ));
    }

    scene(sid, t0, len, layers, track.motions)
}

fn lower_vitruvian(cx: &Ctx, comp_id: &str, sid: &str, t0: f64, len: f64, z: i32) -> Scene {
    let mut layers = Vec::new();
    let mut track = Track::default();
    let roles = cx.roles(comp_id);
    let ink = cx.paint(comp_id, Swatch::Ink, Use::Line { meaning: true });
    let accent = cx.paint(comp_id, Swatch::Accent, Use::Line { meaning: true });

    let sz = (cx.short * 0.72).round();
    let pad = sz * 0.08;
    let s1 = sz - pad;
    let sq_d = format!(
        "M{pad:.1},{pad:.1} L{s1:.1},{pad:.1} L{s1:.1},{s1:.1} L{pad:.1},{s1:.1} Z M{pad:.1},{:.1} L{s1:.1},{:.1} M{:.1},{pad:.1} L{:.1},{s1:.1}",
        sz * 0.54,
        sz * 0.46,
        sz * 0.36,
        sz * 0.64
    );
    let sw = (cx.short * 0.006).round().max(4.0);
    let sq_lid = format!("{sid}.frame");
    layers.push(layer(
        &sq_lid,
        LayerKind::Path {
            d: sq_d,
            fill: None,
            stroke: Some(Stroke {
                color: roles.structure_strong,
                width: sw,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [sz, sz],
        z + 1,
    ));

    let ring_lid = format!("{sid}.circle");
    layers.push(layer(
        &ring_lid,
        LayerKind::Path {
            d: ellipse_path(sz * 0.82, sz * 0.82),
            fill: None,
            stroke: Some(Stroke {
                color: ink,
                width: (cx.short * 0.022).round().max(12.0),
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0, cx.h / 2.0],
        [sz * 0.82, sz * 0.82],
        z + 2,
    ));
    track.push(
        &ring_lid,
        0.0,
        (len * 0.75).max(0.2),
        Shape::Ease(Easing::OutQuint),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    );

    let dot_sz = (cx.short * 0.032).round();
    let dlid = format!("{sid}.dot");
    layers.push(layer(
        &dlid,
        LayerKind::Path {
            d: ellipse_path(dot_sz, dot_sz),
            fill: Some(motion_core::scene::Fill::Solid(accent)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [cx.w / 2.0 + sz * 0.24, cx.h / 2.0 + sz * 0.29],
        [dot_sz, dot_sz],
        z + 3,
    ));
    scene(sid, t0, len, layers, track.motions)
}

/// Resolves `beat` and `beat_cuts` on [`PhotoStack`] steps to component-relative seconds.
pub(crate) fn resolve_steps(
    ps: &mut PhotoStack,
    first_beat: Option<u32>,
    beats: &[f64],
    start: f64,
    fps: f64,
) {
    let lead = crate::taste::CUT_LEAD_FRAMES / fps;
    let snap = |t: f64| ((t * fps).round() / fps - lead).max(0.0);
    if let (Some(first), Some(every)) = (first_beat, ps.beat_cuts) {
        let every = every.max(1);
        for (i, s) in ps.steps.iter_mut().enumerate() {
            if s.beat.is_none() && (i == 0 || s.at == 0.0) {
                s.beat = Some(first + every * i as u32);
            }
        }
    }
    for s in &mut ps.steps {
        if let Some(t) = s.beat.and_then(|n| beats.get(n as usize)) {
            s.at = ((snap(*t) - start).max(0.0) * 1e6).round() / 1e6;
            s.beat = None;
        }
    }
}

/// Program-second event times emitted by a [`PhotoStack`] for `beat_coverage`.
pub(crate) fn step_events(ps: &PhotoStack, start: f64, end: f64) -> Vec<f64> {
    let length = (end - start).max(0.04);
    let n = ps.steps.len();
    let mut out = Vec::new();
    for (i, s) in ps.steps.iter().enumerate() {
        let s_start = s.at.clamp(0.0, (length - 0.04).max(0.0));
        let s_end = if i + 1 < n {
            ps.steps[i + 1].at.clamp(s_start + 0.04, length)
        } else {
            length
        };
        let s_len = (s_end - s_start).max(0.04);
        out.push(start + s_start);
        if matches!(s.layout, PhotoLayout::WordMorph)
            && (s.to_text.is_some() || (s.subtext.is_some() && s_len >= 0.5))
        {
            out.push(start + s_start + s_len * 0.5);
        }
    }
    out
}

pub(crate) fn lower(cx: &Ctx, c: &Component, ps: &PhotoStack, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let mut scenes = Vec::new();
    let n = ps.steps.len();
    for (i, step) in ps.steps.iter().enumerate() {
        let s_start = r6(step.at.clamp(0.0, (length - 0.04).max(0.0)));
        let s_end = if i + 1 < n {
            r6(ps.steps[i + 1].at.clamp(s_start + 0.04, length))
        } else {
            r6(length)
        };
        let s_len = r6((s_end - s_start).max(0.04));
        let t0 = r6(c.start + s_start);
        let sid = format!("{id}.s{i}");
        let seed = (i as u64 + 1) * 17;

        match step.layout {
            PhotoLayout::BrandMark => {
                scenes.push(lower_brand_mark(cx, id, &sid, step, t0, s_len, z, i));
            }
            PhotoLayout::WordMorph => {
                let w0 = step.text.as_deref().unwrap_or("ask");
                let stroke = step.stroke.unwrap_or(AccentStroke::TopSlash);
                if let Some(w1) = step.to_text.as_deref() {
                    let half = r6((s_len * 0.5).max(0.04));
                    let rem = r6((s_len - half).max(0.04));
                    scenes.push(lower_word_phase(
                        cx,
                        id,
                        &format!("{sid}_a"),
                        w0,
                        None,
                        &step.assets,
                        stroke,
                        step.ruled,
                        false,
                        t0,
                        half,
                        z,
                        seed,
                    ));
                    scenes.push(lower_word_phase(
                        cx,
                        id,
                        &format!("{sid}_b"),
                        w1,
                        step.subtext.as_deref(),
                        &step.assets,
                        stroke,
                        step.ruled,
                        true,
                        r6(t0 + half),
                        rem,
                        z,
                        seed + 3,
                    ));
                } else if step.subtext.is_some() && s_len >= 0.5 {
                    let half = r6((s_len * 0.5).max(0.04));
                    let rem = r6((s_len - half).max(0.04));
                    scenes.push(lower_word_phase(
                        cx,
                        id,
                        &format!("{sid}_a"),
                        w0,
                        None,
                        &step.assets,
                        stroke,
                        step.ruled,
                        false,
                        t0,
                        half,
                        z,
                        seed,
                    ));
                    scenes.push(lower_word_phase(
                        cx,
                        id,
                        &format!("{sid}_b"),
                        w0,
                        step.subtext.as_deref(),
                        &step.assets,
                        stroke,
                        step.ruled,
                        true,
                        r6(t0 + half),
                        rem,
                        z,
                        seed + 3,
                    ));
                } else {
                    scenes.push(lower_word_phase(
                        cx,
                        id,
                        &sid,
                        w0,
                        step.subtext.as_deref(),
                        &step.assets,
                        stroke,
                        step.ruled,
                        false,
                        t0,
                        s_len,
                        z,
                        seed,
                    ));
                }
            }
            PhotoLayout::Grid => {
                scenes.push(lower_grid(cx, id, &sid, step, t0, s_len, z, seed));
            }
            PhotoLayout::Zoetrope => {
                scenes.push(lower_zoetrope(cx, id, &sid, t0, s_len, z));
            }
            PhotoLayout::Hero => {
                scenes.push(lower_hero(cx, id, &sid, step, ps.slot, t0, s_len, z, seed));
            }
            PhotoLayout::Blueprint => {
                scenes.push(lower_blueprint(cx, id, &sid, t0, s_len, z));
            }
            PhotoLayout::Starburst3d => {
                scenes.push(lower_starburst_3d(cx, id, &sid, t0, s_len, z));
            }
            PhotoLayout::Constellation => {
                scenes.push(lower_constellation(cx, id, &sid, t0, s_len, z));
            }
            PhotoLayout::Vitruvian => {
                scenes.push(lower_vitruvian(cx, id, &sid, t0, s_len, z));
            }
        }
    }
    scenes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orrery_engraving_program_compiles_and_passes_qa() {
        let json = include_str!("../../../examples/v2/orrery_engraving.program.json");
        let beats: Vec<f64> = (0..36).map(|i| i as f64 * (60.0 / 112.5)).collect();
        let downbeats: Vec<f64> = beats.iter().copied().step_by(4).collect();
        let ts = crate::Typesetter::approx();
        let mut prog = MotionProgram::from_json(json).expect("valid program JSON");
        prog.resolve_beats(&beats);
        crate::validate(&prog).expect("program validates");
        let bc = crate::qa::beat_coverage(&prog, &beats, &downbeats);
        assert!(bc.passes(), "expected beat_coverage pass: {}", bc.report());
        let project = crate::lower(&prog, None, &ts);
        motion_core::validate(&project, None).expect("lowered MotionProject validates");
    }

    #[test]
    fn unknown_cutout_rejected_by_check() {
        let ps = PhotoStack {
            preset: PhotoPreset::EngravingRed,
            slot: crate::program::Slot::Center,
            beat_cuts: None,
            steps: vec![PhotoStep {
                at: 0.0,
                beat: None,
                layout: PhotoLayout::Hero,
                asset: Some("not_a_real_cutout".into()),
                assets: Vec::new(),
                text: None,
                to_text: None,
                subtext: None,
                stroke: None,
                ruled: false,
            }],
        };
        let errs = ps.check(2.0);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].1.contains("unknown bundled cutout"));
    }

    #[test]
    fn bundled_cutout_metrics_32_entries_all_valid() {
        assert_eq!(super::BUNDLED_CUTOUT_METRICS.len(), 37);
        let mut seen = std::collections::BTreeSet::new();
        for m in &super::BUNDLED_CUTOUT_METRICS {
            assert!(
                seen.insert(m.name),
                "duplicate cutout name in table: {}",
                m.name
            );
            assert!(!m.name.is_empty(), "cutout name must not be empty");
            let [x, y, w, h] = m.alpha_bbox;
            assert!(
                w > 0.0 && h > 0.0,
                "{}: alpha_bbox width and height must be positive",
                m.name
            );
            assert!(
                x >= 0.0 && y >= 0.0,
                "{}: alpha_bbox origin must be non-negative",
                m.name
            );
            assert!(
                x + w <= 1.0001,
                "{}: alpha_bbox extends beyond right edge: {}",
                m.name,
                x + w
            );
            assert!(
                y + h <= 1.0001,
                "{}: alpha_bbox extends beyond bottom edge: {}",
                m.name,
                y + h
            );
            assert!(
                super::cutout_metrics(m.name).is_some(),
                "{}: lookup via cutout_metrics failed",
                m.name
            );
            assert!(
                super::resolve_cutout(m.name).is_some(),
                "{}: lookup via resolve_cutout failed",
                m.name
            );
        }
    }

    #[test]
    fn hero_and_word_phase_have_no_fake_frame_marks() {
        let json = include_str!("../../../examples/v2/orrery_engraving.program.json");
        let ts = crate::Typesetter::approx();
        let prog = MotionProgram::from_json(json).expect("valid JSON");
        let project = crate::lower(&prog, None, &ts);
        for s in &project.scenes {
            for l in &s.layers {
                assert!(
                    !l.id.contains("frame_mark"),
                    "found forbidden frame_mark layer: {}",
                    l.id
                );
            }
        }
    }
}
