# test-premium-ui

A single visual integration test for Vieww's premium UI surface. It deliberately composes effects, typography, controls, vectors, data visualisation and motion into one application-shaped frame instead of proving each primitive in isolation.

## Run

```sh
cargo run --release -p test-premium-ui -- /tmp/vieww-premium-test
```

Outputs:

- `test-premium-ui.gif` — the review artifact; inspect the entire sequence.
- `test-premium-ui-first.png` — initial frame.
- `test-premium-ui-last.png` — final frame.

The test exits non-zero if any layout overflow is reported across the sequence or if the rendered sequence barely changes.

## What to inspect visually

The left rail tests dense hierarchy and navigation composition. The KPI row tests repeated card layout, typography, progress and elevation. The performance card tests charts, custom paint, labels, clipping and transforms. The controls card tests switches, checkboxes, sliders, determinate progress, buttons and a backdrop-filtered glass surface. The overlay card tests stacking and deliberate positioning. The background and vignette test large gradients and translucent layers.

The source uses deterministic `t` sampling instead of a live wall-clock so a particular frame can always be reproduced.

## Important provenance

The archive uploaded with this review contained the repository shell but not the `crates/` source tree. The test was therefore authored against the complete `vieww_beta_main(3).tar` source archive already present in the project Library. Before merging, run the test against the exact checkout you intend to ship.
