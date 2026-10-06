//! Image treatment choice (0.5): which preset an image gets (core-owned
//! mapping from style + role) and the preset's explicit parameters.
//! Spec: docs/IMAGE_TREATMENTS.md.

use super::subject_rules::{
    needs_contrast_treatment, outline_color, treated_color, KEYLINE_WIDTH_U, STICKER_WIDTH_U,
};
use super::taste::{tone_image_treatment, ImageTreatmentBias, ResolvedStyleProfile};
use super::theme::Palette;
use crate::assets::{AssetRole, ManifestEntry};
use crate::scene::{
    Color, ContactShadow, Duotone, ImageTreatment, Layer, LayerKind, PaperEdge, Sticker, TintSpec,
    TreatmentPreset,
};
use crate::style::{MaterialStyle, StyleFamily, StyleProfile, TextureStyle, Tone};

/// (0.7) The preset for an image in `role` under the resolved taste. When the
/// resolved bias is the tone's own bias the 0.6 legacy-aware mapping
/// ([`preset_for`]) decides, byte-identically; a bias that came from a
/// reference uses the direct bias table. People are never duotoned.
pub(crate) fn preset_for_taste(
    taste: &ResolvedStyleProfile,
    style: &StyleProfile,
    role: AssetRole,
) -> TreatmentPreset {
    if taste.image_treatment == tone_image_treatment(taste.tone) {
        return preset_for(style, role);
    }
    let person = matches!(role, AssetRole::HeroSubject | AssetRole::Portrait);
    match (taste.image_treatment, role) {
        (ImageTreatmentBias::Classic, _) => preset_for(style, role),
        (ImageTreatmentBias::Natural, _) => TreatmentPreset::Natural,
        (ImageTreatmentBias::Muted, _) => TreatmentPreset::MutedDocumentary,
        (ImageTreatmentBias::Monochrome, AssetRole::EvidenceImage) => {
            TreatmentPreset::MutedDocumentary
        }
        (ImageTreatmentBias::Monochrome, _) => TreatmentPreset::EditorialMonochrome,
        (ImageTreatmentBias::Duotone, _) if person => TreatmentPreset::MutedDocumentary,
        (ImageTreatmentBias::Duotone, AssetRole::EvidenceImage) => {
            TreatmentPreset::MutedDocumentary
        }
        (ImageTreatmentBias::Duotone, _) => TreatmentPreset::EditorialDuotone,
        (ImageTreatmentBias::PaperCutout, AssetRole::Environment) => {
            TreatmentPreset::EditorialDuotone
        }
        (ImageTreatmentBias::PaperCutout, AssetRole::EvidenceImage) => {
            TreatmentPreset::MutedDocumentary
        }
        (ImageTreatmentBias::PaperCutout, _) => TreatmentPreset::PaperCutout,
        (ImageTreatmentBias::PrintCutout, AssetRole::Environment) => {
            TreatmentPreset::EditorialDuotone
        }
        (ImageTreatmentBias::PrintCutout, AssetRole::EvidenceImage) => TreatmentPreset::Natural,
        (ImageTreatmentBias::PrintCutout, _) => TreatmentPreset::PaperCutout,
    }
}

/// The preset for an image in `role` under `style`. People are never duotoned
/// (the style has no explicit duotone request); backgrounds recede.
pub(crate) fn preset_for(style: &StyleProfile, role: AssetRole) -> TreatmentPreset {
    // (0.6) ImageTreatmentDirector bias: `style` is the effective style, whose
    // tone is the resolved tone (`auto` = classic mapping below).
    match (style.tone, role) {
        (Tone::Technical, AssetRole::EvidenceImage) => return TreatmentPreset::MutedDocumentary,
        (Tone::Technical, _) => return TreatmentPreset::EditorialMonochrome,
        (Tone::Playful, AssetRole::Environment) => return TreatmentPreset::EditorialDuotone,
        (Tone::Playful, AssetRole::EvidenceImage) => return TreatmentPreset::Natural,
        (Tone::Playful, _) => return TreatmentPreset::PaperCutout,
        _ => {}
    }
    let heavy = style.texture_style == TextureStyle::HeavyPrint;
    let collage_paper =
        style.family == StyleFamily::EditorialCollage && style.material == MaterialStyle::Paper;
    match role {
        AssetRole::Environment => {
            if heavy {
                TreatmentPreset::EditorialMonochrome
            } else {
                TreatmentPreset::EditorialDuotone
            }
        }
        AssetRole::EvidenceImage => TreatmentPreset::MutedDocumentary,
        AssetRole::HeroSubject
        | AssetRole::Portrait
        | AssetRole::HeroObject
        | AssetRole::SupportingObject
        | AssetRole::TransitionObject
        | AssetRole::ForegroundOccluder => {
            if heavy {
                TreatmentPreset::EditorialMonochrome
            } else if collage_paper {
                TreatmentPreset::PaperCutout
            } else if style.texture_style == TextureStyle::None {
                TreatmentPreset::Natural
            } else {
                TreatmentPreset::MutedDocumentary
            }
        }
    }
}

/// Explicit parameters for `preset`. `alpha` = the image is a cutout (edge and
/// shadow only apply to cutouts); `u` = canvas unit (px per 1080-px reference).
pub(crate) fn treatment_for(
    preset: TreatmentPreset,
    palette: &Palette,
    seed: u64,
    u: f32,
    alpha: bool,
) -> ImageTreatment {
    let ink = palette.ink;
    let mut t = ImageTreatment {
        preset,
        seed,
        ..ImageTreatment::natural()
    };
    let shadow = |a: u8, offset: [f32; 2], blur: f32| ContactShadow {
        color: ink.with_alpha(a),
        offset: [offset[0] * u, offset[1] * u],
        blur: blur * u,
    };
    match preset {
        TreatmentPreset::Natural => {}
        TreatmentPreset::EditorialMonochrome => {
            t.desaturate = 1.0;
            t.brightness = 0.02;
            t.contrast = 1.18;
            t.tint = Some(TintSpec {
                color: ink,
                amount: 0.06,
            });
            t.grain = 0.06;
            if alpha {
                t.shadow = Some(shadow(0x30, [0.0, 8.0], 12.0));
            }
        }
        TreatmentPreset::EditorialDuotone => {
            t.contrast = 1.08;
            t.duotone = Some(Duotone {
                shadow: ink,
                highlight: palette.paper,
                amount: 0.9,
            });
            t.grain = 0.05;
        }
        TreatmentPreset::PaperCutout => {
            t.desaturate = 0.18;
            t.brightness = 0.01;
            t.contrast = 1.06;
            t.tint = Some(TintSpec {
                color: palette.paper,
                amount: 0.05,
            });
            t.grain = 0.04;
            if alpha {
                t.edge = Some(PaperEdge {
                    color: palette.card,
                    width: 7.0 * u,
                });
                t.shadow = Some(shadow(0x38, [5.0, 9.0], 14.0));
            }
        }
        TreatmentPreset::MutedDocumentary => {
            t.desaturate = 0.35;
            t.contrast = 0.94;
            t.tint = Some(TintSpec {
                color: palette.paper,
                amount: 0.08,
            });
            t.grain = 0.035;
            if alpha {
                t.shadow = Some(shadow(0x22, [0.0, 6.0], 16.0));
            }
        }
    }
    t
}

/// (0.9) Ink-on-ground treatment for monochrome line-art cutouts: the preset's
/// treatment with the artwork recoloured to the palette ink (so it reads on any
/// ground) and every effect that would muddy thin strokes removed. Preset and
/// seed are kept.
pub(crate) fn monochrome_ink(t: ImageTreatment, palette: &Palette) -> ImageTreatment {
    ImageTreatment {
        desaturate: 1.0,
        tint: Some(TintSpec {
            color: palette.ink,
            amount: 1.0,
        }),
        duotone: None,
        grain: 0.0,
        edge: None,
        shadow: None,
        ..t
    }
}

/// (0.23 A5a) The contrast guard of a picture the compiler places with no
/// treatment of its own: a library object placed by `recipes::place_subject`
/// (a delivered image guards itself in `plate::image_layer`). When the
/// picture's mean colour (`entry.analysis`) has under
/// `ASSET_GROUND_MIN_CONTRAST` against `ground`, the layer gets what
/// `subject_rules::guard_contrast` gives a delivered one: a sticker in paper
/// or ink for a cut-out, a keyline for an opaque image. Only an `Image` layer
/// whose treatment is `None` is touched, and only when the rule fails, so a
/// picture that already reads is left as it was. Returns whether it changed
/// the layer.
pub(crate) fn guard_on_ground(
    layer: &mut Layer,
    entry: &ManifestEntry,
    ground: Color,
    palette: &Palette,
    u: f32,
) -> bool {
    let LayerKind::Image { treatment, .. } = &mut layer.kind else {
        return false;
    };
    if treatment.is_some() {
        return false;
    }
    let Some(mean) = entry.analysis.as_ref().and_then(|a| a.mean_color) else {
        return false;
    };
    let natural = ImageTreatment::natural();
    let subject = treated_color(mean, &natural);
    if !needs_contrast_treatment(subject, &[ground]) {
        return false;
    }
    let color = outline_color(subject, palette.paper, palette.ink);
    let mut guarded = natural;
    if entry.alpha {
        guarded.sticker = Some(Sticker {
            color,
            width_px: STICKER_WIDTH_U * u,
        });
    } else {
        guarded.edge = Some(PaperEdge {
            color,
            width: KEYLINE_WIDTH_U * u,
        });
    }
    *treatment = Some(guarded);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette::for_style(&StyleProfile::default())
    }

    fn t(preset: TreatmentPreset, alpha: bool) -> ImageTreatment {
        treatment_for(preset, &pal(), 42, 2.0, alpha)
    }

    #[test]
    fn natural_is_neutral_but_keeps_seed_and_preset() {
        let n = t(TreatmentPreset::Natural, true);
        let mut expect = ImageTreatment::natural();
        expect.seed = 42;
        assert_eq!(n, expect);
    }

    #[test]
    fn editorial_monochrome_matches_table() {
        let p = pal();
        let x = t(TreatmentPreset::EditorialMonochrome, true);
        assert_eq!(x.preset, TreatmentPreset::EditorialMonochrome);
        assert_eq!(x.seed, 42);
        assert_eq!(
            (x.desaturate, x.brightness, x.contrast, x.grain),
            (1.0, 0.02, 1.18, 0.06)
        );
        assert_eq!(
            x.tint,
            Some(TintSpec {
                color: p.ink,
                amount: 0.06
            })
        );
        assert!(x.duotone.is_none() && x.edge.is_none());
        let s = x.shadow.expect("shadow");
        assert_eq!(s.color, p.ink.with_alpha(0x30));
        assert_eq!((s.offset, s.blur), ([0.0, 16.0], 24.0));
    }

    #[test]
    fn editorial_duotone_matches_table() {
        let p = pal();
        let x = t(TreatmentPreset::EditorialDuotone, true);
        assert_eq!(
            (x.desaturate, x.brightness, x.contrast, x.grain),
            (0.0, 0.0, 1.08, 0.05)
        );
        assert_eq!(
            x.duotone,
            Some(Duotone {
                shadow: p.ink,
                highlight: p.paper,
                amount: 0.9
            })
        );
        assert!(x.tint.is_none() && x.edge.is_none() && x.shadow.is_none());
    }

    #[test]
    fn paper_cutout_matches_table() {
        let p = pal();
        let x = t(TreatmentPreset::PaperCutout, true);
        assert_eq!(
            (x.desaturate, x.brightness, x.contrast, x.grain),
            (0.18, 0.01, 1.06, 0.04)
        );
        assert_eq!(
            x.tint,
            Some(TintSpec {
                color: p.paper,
                amount: 0.05
            })
        );
        let e = x.edge.expect("edge");
        assert_eq!((e.color, e.width), (p.card, 14.0));
        let s = x.shadow.expect("shadow");
        assert_eq!(s.color, p.ink.with_alpha(0x38));
        assert_eq!((s.offset, s.blur), ([10.0, 18.0], 28.0));
    }

    #[test]
    fn muted_documentary_matches_table() {
        let p = pal();
        let x = t(TreatmentPreset::MutedDocumentary, true);
        assert_eq!(
            (x.desaturate, x.brightness, x.contrast, x.grain),
            (0.35, 0.0, 0.94, 0.035)
        );
        assert_eq!(
            x.tint,
            Some(TintSpec {
                color: p.paper,
                amount: 0.08
            })
        );
        assert!(x.edge.is_none());
        let s = x.shadow.expect("shadow");
        assert_eq!(s.color, p.ink.with_alpha(0x22));
        assert_eq!((s.offset, s.blur), ([0.0, 12.0], 32.0));
    }

    #[test]
    fn opaque_images_never_get_edge_or_shadow() {
        for preset in [
            TreatmentPreset::Natural,
            TreatmentPreset::EditorialMonochrome,
            TreatmentPreset::EditorialDuotone,
            TreatmentPreset::PaperCutout,
            TreatmentPreset::MutedDocumentary,
        ] {
            let x = t(preset, false);
            assert!(x.edge.is_none() && x.shadow.is_none(), "{preset:?}");
            // Colour ops are unchanged by `alpha`.
            let c = t(preset, true);
            assert_eq!(
                (x.desaturate, x.contrast, x.grain, x.tint, x.duotone),
                (c.desaturate, c.contrast, c.grain, c.tint, c.duotone)
            );
        }
    }

    #[test]
    fn deterministic_and_scales_with_u() {
        let a = treatment_for(TreatmentPreset::PaperCutout, &pal(), 7, 1.0, true);
        let b = treatment_for(TreatmentPreset::PaperCutout, &pal(), 7, 1.0, true);
        assert_eq!(a, b);
        assert_eq!(a.edge.map(|e| e.color), Some(Color::rgb(0xF6, 0xF1, 0xE6)));
        assert_eq!(a.edge.map(|e| e.width), Some(7.0));
    }

    /// A manifest entry with the analysis the guard reads: `mean` colour.
    fn entry(alpha: bool, mean: [u8; 3]) -> ManifestEntry {
        serde_json::from_value(serde_json::json!({
            "id": "beat_1.supporting_object", "path": "library/x/y.png",
            "width": 100, "height": 100, "alpha": alpha,
            "analysis": {
                "subject_bounds": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0},
                "edges": {"top": false, "bottom": false, "left": false, "right": false},
                "coverage": 0.5,
                "occupancy": {"cols": 1, "rows": ["f"]},
                "safe_regions": [],
                "mean_color": mean,
            },
        }))
        .expect("entry")
    }

    fn picture(treatment: Option<ImageTreatment>) -> Layer {
        crate::compiler::base_layer(
            "b1.hero.x".to_string(),
            (0.0, 0.0, 100.0, 100.0),
            LayerKind::Image {
                asset: "asset.x".to_string(),
                fit: crate::scene::Fit::Contain,
                treatment,
                playback: None,
                insert: None,
            },
            20,
        )
    }

    fn treatment_of(l: &Layer) -> Option<&ImageTreatment> {
        match &l.kind {
            LayerKind::Image { treatment, .. } => treatment.as_ref(),
            _ => None,
        }
    }

    #[test]
    fn a_picture_that_vanishes_into_its_ground_gets_a_sticker_or_a_keyline() {
        let p = pal();
        // A mid-tone orange on a mid-tone teal: well under 2:1.
        let teal = Color::rgb(0x7D, 0xD3, 0xC0);
        let orange = [0xE2, 0x73, 0x3A];
        let mut cutout = picture(None);
        assert!(guard_on_ground(
            &mut cutout,
            &entry(true, orange),
            teal,
            &p,
            2.0
        ));
        let sticker = treatment_of(&cutout)
            .and_then(|t| t.sticker)
            .expect("sticker");
        assert_eq!(sticker.width_px, STICKER_WIDTH_U * 2.0);
        assert!(
            crate::compiler::explore::contrast_ratio(
                Color::rgb(orange[0], orange[1], orange[2]),
                sticker.color
            ) >= crate::compiler::taste_rules::ASSET_GROUND_MIN_CONTRAST
        );
        // An opaque picture gets a keyline instead.
        let mut opaque = picture(None);
        assert!(guard_on_ground(
            &mut opaque,
            &entry(false, orange),
            teal,
            &p,
            1.0
        ));
        let edge = treatment_of(&opaque).and_then(|t| t.edge).expect("keyline");
        assert_eq!(edge.width, KEYLINE_WIDTH_U);
    }

    #[test]
    fn a_picture_that_reads_or_already_has_a_treatment_is_left_alone() {
        let p = pal();
        let teal = Color::rgb(0x7D, 0xD3, 0xC0);
        let orange = [0xE2, 0x73, 0x3A];
        // Near-black on teal reads.
        let mut dark = picture(None);
        assert!(!guard_on_ground(
            &mut dark,
            &entry(true, [0x20; 3]),
            teal,
            &p,
            1.0
        ));
        assert!(treatment_of(&dark).is_none());
        // A treated picture is never touched, nor one with no colour analysis.
        let mut treated = picture(Some(ImageTreatment::natural()));
        assert!(!guard_on_ground(
            &mut treated,
            &entry(true, orange),
            teal,
            &p,
            1.0
        ));
        assert_eq!(treatment_of(&treated), Some(&ImageTreatment::natural()));
        let mut unknown = picture(None);
        let mut e = entry(true, orange);
        e.analysis = None;
        assert!(!guard_on_ground(&mut unknown, &e, teal, &p, 1.0));
    }

    #[test]
    fn monochrome_ink_recolours_to_palette_ink() {
        let p = pal();
        for preset in [
            TreatmentPreset::Natural,
            TreatmentPreset::EditorialMonochrome,
            TreatmentPreset::EditorialDuotone,
            TreatmentPreset::PaperCutout,
            TreatmentPreset::MutedDocumentary,
        ] {
            let x = monochrome_ink(t(preset, true), &p);
            assert_eq!(x.preset, preset);
            assert_eq!(x.seed, 42);
            assert_eq!(x.desaturate, 1.0);
            assert_eq!(
                x.tint,
                Some(TintSpec {
                    color: p.ink,
                    amount: 1.0
                })
            );
            assert!(x.duotone.is_none() && x.edge.is_none() && x.shadow.is_none());
            assert_eq!(x.grain, 0.0);
        }
    }
}
