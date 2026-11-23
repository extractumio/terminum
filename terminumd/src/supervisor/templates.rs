// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-263: project-type starters contain only editable preferences.

use std::collections::BTreeMap;

use serde::Serialize;

use super::scoreboard::Key;

#[derive(Serialize)]
struct Surfaces {
    t2: &'static [&'static str],
    t3: &'static [&'static str],
}

#[derive(Serialize)]
struct Probes {
    git: bool,
    gate_record: bool,
}

#[derive(Serialize)]
pub(super) struct Template {
    id: &'static str,
    name: &'static str,
    never: &'static [&'static str],
    priorities: &'static [&'static str],
    targets: BTreeMap<Key, &'static str>,
    surfaces: Surfaces,
    probes: Probes,
}

pub(super) fn choices() -> [Template; 4] {
    let targets = |goals| Key::ALL.into_iter().zip(goals).collect();
    [
        Template {
            id: "open_source_tool",
            name: "Open-source tool",
            never: &["Expose a contributor's private data.", "Break a documented interface without a migration path."],
            priorities: &["Keep documented interfaces stable.", "Make contributions and releases repeatable."],
            targets: targets([
                "Increase completed improvements per maintainer hour.",
                "Keep release and nightly regressions at zero.",
                "Ship ready fixes within the next planned release.",
                "Publish at least one release per month when fixes are ready.",
                "Clear critical regressions before taking new features.",
                "Batch routine maintainer decisions into the weekly review.",
                "Automate recurring release and triage work.",
            ]),
            surfaces: Surfaces {
                t2: &["docs/api/**", "api/**"],
                t3: &["install/**", "release/**"],
            },
            probes: Probes { git: true, gate_record: true },
        },
        Template {
            id: "commercial_app",
            name: "Commercial app",
            never: &["Expose customer data.", "Roll out a data migration without a recovery plan."],
            priorities: &["Protect customer data and availability.", "Reduce friction in the main customer workflow."],
            targets: targets([
                "Increase shipped customer improvements per owner hour.",
                "Keep customer data incidents at zero.",
                "Release critical fixes within one day.",
                "Ship safe, small improvements weekly.",
                "Resolve urgent customer problems before planned work.",
                "Reserve urgent interruptions for issues affecting customers.",
                "Reduce manual rollout and support work.",
            ]),
            surfaces: Surfaces {
                t2: &["api/**", "integrations/**"],
                t3: &["auth/**", "billing/**", "migrations/**"],
            },
            probes: Probes { git: true, gate_record: true },
        },
        Template {
            id: "prototype",
            name: "Prototype",
            never: &["Use real customer data in experiments.", "Make an irreversible commitment before validating the idea."],
            priorities: &["Test the riskiest assumption first.", "Keep the next experiment cheap to change."],
            targets: targets([
                "Finish more experiments per owner hour.",
                "Keep the demonstration usable for each trial.",
                "Get a testable result within two working days.",
                "Share a runnable build at each experiment checkpoint.",
                "Keep only the next experiments on the active backlog.",
                "Group routine choices into the experiment review.",
                "Keep setup and reset repeatable.",
            ]),
            surfaces: Surfaces {
                t2: &["experiments/interfaces/**"],
                t3: &["fixtures/shared/**"],
            },
            probes: Probes { git: true, gate_record: false },
        },
        Template {
            id: "internal_tool",
            name: "Internal tool",
            never: &["Bypass access controls.", "Make an irreversible change to shared records without approval."],
            priorities: &["Keep staff workflows reliable.", "Reduce repeated manual work."],
            targets: targets([
                "Increase shipped staff improvements per owner hour.",
                "Keep interruptions to core staff workflows at zero.",
                "Ship urgent workflow fixes within two working days.",
                "Deliver useful improvements at least monthly.",
                "Review blocked staff requests every week.",
                "Group maintenance decisions into the weekly review.",
                "Reduce manual support and administration work.",
            ]),
            surfaces: Surfaces {
                t2: &["integrations/**", "reports/**"],
                t3: &["access/**", "migrations/**"],
            },
            probes: Probes { git: true, gate_record: true },
        },
    ]
}
