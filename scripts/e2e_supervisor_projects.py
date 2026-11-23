#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-263: different project templates and one critical line on a private terminumd."""
from contextlib import closing
import json
from pathlib import Path
import shlex
import sqlite3
import time

from e2e_supervisor_settings import Fixture
from e2e_ws import Terminumd, check, summary, wait_for
from testrun import free_port, own


def attach(fixture, paths):
    expected = ["open_source_tool", "commercial_app", "prototype", "internal_tool"]
    first = fixture.request(paths[0] + "/settings")
    templates = {entry["id"]: entry for entry in first["templates"]}
    check("AC-263 the four templates offer editable settings without command authority",
          list(templates) == expected and all(
              set(entry) == {"id", "name", "never", "priorities", "targets", "surfaces", "probes"}
              and entry["never"] and entry["priorities"] and len(entry["targets"]) == 7
              and entry["surfaces"]["t2"] and entry["surfaces"]["t3"]
              and set(entry["probes"]) == {"git", "gate_record"}
              for entry in templates.values()))
    check("AC-263 viewing templates leaves the saved pack and switch alone",
          first["revision"] == 0 and not first["settings"]["charter"]["never"]
          and not fixture.request(paths[0])["enabled"])

    calibrated = []
    for path, name in zip(paths, ["open_source_tool", "prototype"]):
        view = fixture.request(path + "/settings")
        saved, template = view["settings"], templates[name]
        charter = saved["charter"]
        for key in ["never", "priorities"]:
            charter[key] = template[key]
        charter["indicator_targets"] = {**template["targets"], "stability": f"Review {name} stability every week."}
        charter["surfaces"].update(template["surfaces"])
        saved["probes"].update(template["probes"])
        view = fixture.request(path + "/settings", "PUT", {"revision": view["revision"], "settings": saved})
        check(f"AC-263 the {name} preferences save with an owner edit and no activation or commands",
              view["settings"] == saved and not view["confirmed_probes"] and not view["unset_probes"]
              and not fixture.request(path)["enabled"])
        scan = fixture.scan(path)
        calibrated.append(scan["detectors"]["flow.wait_human_age"]["observed_thresholds"]["wait_minutes"])
        indicators = fixture.request(path + "/screen")["indicators"]
        check(f"AC-263 {name} attaches with its own goals and code-only scan",
              scan["code_only"] and indicators[1]["target"] == charter["indicator_targets"]["stability"]
              and indicators[1]["verdict"] == "not measured yet")
    check("AC-263 different templates leave each project with its own historical baselines",
          calibrated == [{"min": 300, "max": 300}, {"min": 900, "max": 900}])

    rejected = {"version": 1, "settings": {"source_project": paths[0].split("/")[-2],
                                          "baselines": {"private_fixture": 123}}}
    bad = fixture.request(paths[1] + "/import", "POST", {
        "revision": 1, "file": json.dumps(rejected),
    }, expected=400)
    check("AC-263 a pack cannot name another project as a source of retained state",
          bad["error"] == "The file is not a valid Supervisor settings pack."
          and fixture.request(paths[1] + "/settings")["revision"] == 1)


def seed_incident(db, project, level):
    # Synthetic incident inputs for the real queue. Insert the lower-level incident first.
    now = int(time.time())
    fingerprint = f"fixture-{level}"
    incident = db.execute("""INSERT INTO supervisor_incidents
        (project, subject_kind, subject_id, opened_at, last_firing_at)
        VALUES (?, 'project', 'private case fixture', ?, ?)""", (project, now, now)).lastrowid
    db.execute("""INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind,
        subject_id, metric, evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?, ?, 'flow.failed_no_reason', 'project', 'private case fixture', 'fixture_metric',
          '[{"table":"inputs","id":42}]', ?, ?, ?, ?, 1, 'raised')""",
               (fingerprint, project, json.dumps({"value": 5, "baseline": 4, "threshold": 4,
                                                  "window_start": now - 10, "window_end": now}),
                json.dumps({"impact": level, "blast": 0, "reversibility": 0, "confidence": 1}), now, now))
    event = db.execute("""INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, level)
        VALUES (?, ?, 'signal', 'system', ?, ?)""", (incident, now, fingerprint, level)).lastrowid
    db.execute("INSERT INTO supervisor_incident_signals VALUES (?, ?, ?, x'01', 5, 4, ?, 0)",
               (fingerprint, incident, level, event))
    return incident


def shared_line(fixture, paths):
    channel, output = (Path(fixture.server.root, name) for name in ("shared channel", "notices"))
    channel.write_text("#!/bin/sh\ncat >> " + shlex.quote(str(output)) + "\n")
    channel.chmod(0o700)
    low_path, high_path = paths
    with closing(sqlite3.connect(fixture.db)) as db, db:
        low, high = (seed_incident(db, path.split("/")[-2], level)
                     for path, level in [(low_path, 2), (high_path, 3)])
    local = time.localtime()
    minute = local.tm_hour * 60 + local.tm_min
    line = {"channels": [str(channel)], "link_base": None, "ladder": [10, 30, 60],
            "quiet": [(minute - 30) % 1440, (minute + 30) % 1440]}
    saved = fixture.request(low_path + "/line", "PUT", {"revision": 0, "settings": line})
    check("AC-263 both project tabs share the same channels, quiet hours and revision",
          fixture.request(high_path + "/line") == saved)
    check("AC-263 another tab cannot overwrite the shared line with a stale revision",
          "changed in another tab" in fixture.request(high_path + "/line", "PUT", {
              "revision": 0, "settings": line,
          }, expected=400)["error"])
    time.sleep(11)
    with closing(sqlite3.connect(fixture.db)) as db:
        pending = db.execute("SELECT incident FROM supervisor_line_pending ORDER BY id").fetchall()
    check("AC-263 the same quiet window defers both projects' notices",
          not output.exists() and pending == [(low,), (high,)])
    fixture.request(high_path + "/line", "PUT", {"revision": saved["revision"], "settings": {**line, "quiet": None}})
    notices = wait_for(lambda: output.read_text().splitlines() if output.exists() else [],
                       lambda lines: len(lines) >= 2, 20, every=0.25)
    check("AC-263 one executable receives the higher level first, using only the fixed incident fields",
          notices == [f"Critical incident {incident} · Level {level}. Active signals: 1. Acknowledge in Terminum."
                      for incident, level in [(high, 3), (low, 2)]], notices)


def main():
    server = Terminumd(free_port(), own("tm-test-supervisor-projects"))
    try:
        server.start()
        fixture = Fixture(server)
        paths = [fixture.project("API", 300, 28), fixture.project("Lab", 900, 10)]
        attach(fixture, paths)
        shared_line(fixture, paths)
    finally:
        server.cleanup()


if __name__ == "__main__":
    main()
    summary()
