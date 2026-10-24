//! Stagger timing: exact patterns, closed-form cycles, orderings.

use motion_core::motion::stagger::{offsets, ranks, StaggerOrder, StaggerPreset, StaggerSpec};

fn f(frames: u32) -> f64 {
    frames as f64 / 30.0
}

const PRESETS: [StaggerPreset; 3] = [
    StaggerPreset::Calm,
    StaggerPreset::Editorial,
    StaggerPreset::Impact,
];
const ORDERS: [StaggerOrder; 4] = [
    StaggerOrder::Forward,
    StaggerOrder::Reverse,
    StaggerOrder::CenterOut,
    StaggerOrder::EdgesIn,
];

fn spec(preset: StaggerPreset, order: StaggerOrder) -> StaggerSpec {
    StaggerSpec { preset, order }
}

#[test]
fn pattern_values_are_exact() {
    assert_eq!(
        StaggerPreset::Editorial.pattern(),
        (&[0, 4, 7, 13, 18][..], 22)
    );
    assert_eq!(StaggerPreset::Calm.pattern(), (&[0, 6, 11, 19][..], 25));
    assert_eq!(StaggerPreset::Impact.pattern(), (&[0, 3, 5, 10][..], 12));
}

#[test]
fn first_cycle_offsets_match_pattern() {
    let got = offsets(spec(StaggerPreset::Editorial, StaggerOrder::Forward), 5);
    assert_eq!(got, vec![f(0), f(4), f(7), f(13), f(18)]);
    let got = offsets(spec(StaggerPreset::Calm, StaggerOrder::Forward), 4);
    assert_eq!(got, vec![f(0), f(6), f(11), f(19)]);
    let got = offsets(spec(StaggerPreset::Impact, StaggerOrder::Forward), 4);
    assert_eq!(got, vec![f(0), f(3), f(5), f(10)]);
}

#[test]
fn cycle_extension_is_closed_form() {
    // n = 12: three passes of the Impact pattern (4 entries, cycle 12) and
    // 2.4 of the Editorial one.
    let imp = offsets(spec(StaggerPreset::Impact, StaggerOrder::Forward), 12);
    let expect: Vec<f64> = (0..12)
        .map(|r| f([0, 3, 5, 10][r % 4] + (r as u32 / 4) * 12))
        .collect();
    assert_eq!(imp, expect);
    assert_eq!(imp[4], f(12));
    assert_eq!(imp[11], f(10 + 24));

    let ed = offsets(spec(StaggerPreset::Editorial, StaggerOrder::Forward), 12);
    assert_eq!(ed[5], f(22));
    assert_eq!(ed[6], f(26));
    assert_eq!(ed[9], f(18 + 22));
    assert_eq!(ed[10], f(44));
    assert_eq!(ed[11], f(48));

    let calm = offsets(spec(StaggerPreset::Calm, StaggerOrder::Forward), 12);
    assert_eq!(calm[4], f(25));
    assert_eq!(calm[11], f(19 + 50));
}

#[test]
fn forward_and_reverse_ranks() {
    for n in [0usize, 1, 2, 5, 8] {
        assert_eq!(ranks(StaggerOrder::Forward, n), (0..n).collect::<Vec<_>>());
        assert_eq!(
            ranks(StaggerOrder::Reverse, n),
            (0..n).rev().collect::<Vec<_>>()
        );
    }
}

#[test]
fn center_out_odd_and_even() {
    // Odd: the middle is first, then alternating outwards (lower index first).
    assert_eq!(ranks(StaggerOrder::CenterOut, 5), vec![3, 1, 0, 2, 4]);
    assert_eq!(ranks(StaggerOrder::CenterOut, 1), vec![0]);
    // Even: the two middle elements tie; lower index goes first.
    assert_eq!(ranks(StaggerOrder::CenterOut, 4), vec![2, 0, 1, 3]);
    assert_eq!(ranks(StaggerOrder::CenterOut, 6), vec![4, 2, 0, 1, 3, 5]);
    assert_eq!(ranks(StaggerOrder::CenterOut, 2), vec![0, 1]);
}

#[test]
fn edges_in_odd_and_even() {
    // Ends first (lower index first), then inwards.
    assert_eq!(ranks(StaggerOrder::EdgesIn, 5), vec![0, 2, 4, 3, 1]);
    assert_eq!(ranks(StaggerOrder::EdgesIn, 4), vec![0, 2, 3, 1]);
    assert_eq!(ranks(StaggerOrder::EdgesIn, 6), vec![0, 2, 4, 5, 3, 1]);
    assert_eq!(ranks(StaggerOrder::EdgesIn, 1), vec![0]);
    assert_eq!(ranks(StaggerOrder::EdgesIn, 0), Vec::<usize>::new());
}

#[test]
fn ranks_are_permutations() {
    for order in ORDERS {
        for n in 0..40 {
            let mut r = ranks(order, n);
            r.sort_unstable();
            assert_eq!(r, (0..n).collect::<Vec<_>>(), "{order:?} n={n}");
        }
    }
}

#[test]
fn offsets_follow_index_order_under_center_out() {
    // Center element of 5 has rank 0 -> offset 0; the ends come last.
    let o = offsets(spec(StaggerPreset::Editorial, StaggerOrder::CenterOut), 5);
    assert_eq!(o[2], f(0));
    assert_eq!(o[1], f(4));
    assert_eq!(o[3], f(7));
    assert_eq!(o[0], f(13));
    assert_eq!(o[4], f(18));
}

#[test]
fn no_drift_at_high_rank() {
    // rank 1000: Editorial 1000 % 5 = 0, 1000 / 5 = 200 cycles of 22 frames.
    let o = offsets(spec(StaggerPreset::Editorial, StaggerOrder::Forward), 1001);
    assert_eq!(o[1000], f(200 * 22));
    assert_eq!(o[1000], 4400.0 / 30.0);
    assert_eq!(
        StaggerPreset::Editorial.offset_for_rank(1000),
        4400.0 / 30.0
    );
    // Reverse: index 0 has rank 1000.
    let r = offsets(spec(StaggerPreset::Impact, StaggerOrder::Reverse), 1001);
    assert_eq!(r[0], f(250 * 12));
    assert_eq!(r[1000], 0.0);
}

#[test]
fn offsets_non_decreasing_by_rank() {
    for preset in PRESETS {
        for order in ORDERS {
            for n in [1usize, 2, 5, 9, 16, 33] {
                let rk = ranks(order, n);
                let off = offsets(spec(preset, order), n);
                let mut by_rank = vec![0.0; n];
                for i in 0..n {
                    by_rank[rk[i]] = off[i];
                }
                for w in by_rank.windows(2) {
                    assert!(w[0] <= w[1], "{preset:?} {order:?} n={n}: {by_rank:?}");
                }
                if n > 0 {
                    assert_eq!(by_rank[0], 0.0);
                }
            }
        }
    }
}

#[test]
fn offsets_are_deterministic() {
    for preset in PRESETS {
        for order in ORDERS {
            let s = spec(preset, order);
            assert_eq!(offsets(s, 17), offsets(s, 17));
        }
    }
}
