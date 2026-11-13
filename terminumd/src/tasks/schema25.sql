-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- An agent is one project's (terminum-dispatcher.md AC-137): each project has a pool of its own, a name once in it.
-- The one pool's agents each go to one project: the one whose settings name it (a task's Agent, a schedule,
-- Auto-Dispatcher's agents, a queue), then where it ran most, then last; one nothing used goes where most runs were,
-- else to Default. Another project whose settings name it gets a copy under a new id, and those settings name the
-- copy: nothing a human set stops. A run keeps its agent's id, name and command as it started.
CREATE TABLE agent_uses (agent TEXT NOT NULL, project TEXT NOT NULL, named INTEGER NOT NULL, runs INTEGER NOT NULL, last INTEGER NOT NULL, copy TEXT, PRIMARY KEY (agent, project));
INSERT INTO agent_uses (agent, project, named, runs, last)
  SELECT agent, project, max(named), sum(runs), max(last) FROM (
    SELECT agent_id AS agent, project, 0 AS named, count(*) AS runs, max(rowid) AS last FROM runs GROUP BY agent_id, project
    UNION ALL SELECT agent, project, 1, 0, 0 FROM tasks WHERE agent IS NOT NULL AND archived_at IS NULL
    UNION ALL SELECT agent_id, project, 1, 0, 0 FROM schedules WHERE deleted_at IS NULL
    UNION ALL SELECT auto_agent, id, 1, 0, 0 FROM projects WHERE auto_agent IS NOT NULL
    UNION ALL SELECT auto_default, id, 1, 0, 0 FROM projects WHERE auto_default IS NOT NULL
    UNION ALL SELECT agent_id, project, 1, 0, 0 FROM run_waits)
  WHERE agent IN (SELECT id FROM agents) AND project IN (SELECT id FROM projects)
  GROUP BY agent, project;
CREATE TABLE agent_homes (agent TEXT PRIMARY KEY, project TEXT NOT NULL);
INSERT INTO agent_homes (agent, project)
  SELECT a.id, COALESCE(
    (SELECT u.project FROM agent_uses u WHERE u.agent = a.id ORDER BY u.named DESC, u.runs DESC, u.last DESC LIMIT 1),
    (SELECT u.project FROM agent_uses u GROUP BY u.project ORDER BY sum(u.runs) DESC, max(u.last) DESC LIMIT 1),
    (SELECT id FROM projects WHERE is_default = 1))
  FROM agents a;
UPDATE agent_uses SET copy = lower(hex(randomblob(8)))
  WHERE named = 1 AND project <> (SELECT h.project FROM agent_homes h WHERE h.agent = agent_uses.agent);

CREATE TABLE agents_new (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('claude', 'codex')),
  command TEXT NOT NULL,
  effort TEXT NOT NULL DEFAULT '',
  color INTEGER NOT NULL,
  at_once INTEGER NOT NULL DEFAULT 1,
  week_limit REAL,
  month_limit REAL,
  UNIQUE (project, name)
);
INSERT INTO agents_new (id, project, name, kind, command, effort, color, at_once, week_limit, month_limit)
  SELECT a.id, h.project, a.name, a.kind, a.command, a.effort, a.color, a.at_once, a.week_limit, a.month_limit
  FROM agents a JOIN agent_homes h ON h.agent = a.id ORDER BY a.rowid;
INSERT INTO agents_new (id, project, name, kind, command, effort, color, at_once, week_limit, month_limit)
  SELECT u.copy, u.project, a.name, a.kind, a.command, a.effort, a.color, a.at_once, a.week_limit, a.month_limit
  FROM agent_uses u JOIN agents a ON a.id = u.agent WHERE u.copy IS NOT NULL ORDER BY a.rowid, u.project;

UPDATE tasks SET agent = (SELECT u.copy FROM agent_uses u WHERE u.agent = tasks.agent AND u.project = tasks.project)
  WHERE EXISTS (SELECT 1 FROM agent_uses u WHERE u.agent = tasks.agent AND u.project = tasks.project AND u.copy IS NOT NULL);
UPDATE tasks SET agent = NULL WHERE agent IS NOT NULL AND NOT EXISTS (SELECT 1 FROM agents_new n WHERE n.id = tasks.agent AND n.project = tasks.project);
UPDATE schedules SET agent_id = (SELECT u.copy FROM agent_uses u WHERE u.agent = schedules.agent_id AND u.project = schedules.project)
  WHERE EXISTS (SELECT 1 FROM agent_uses u WHERE u.agent = schedules.agent_id AND u.project = schedules.project AND u.copy IS NOT NULL);
UPDATE run_waits SET agent_id = (SELECT u.copy FROM agent_uses u WHERE u.agent = run_waits.agent_id AND u.project = run_waits.project)
  WHERE EXISTS (SELECT 1 FROM agent_uses u WHERE u.agent = run_waits.agent_id AND u.project = run_waits.project AND u.copy IS NOT NULL);
UPDATE projects SET auto_agent = (SELECT u.copy FROM agent_uses u WHERE u.agent = projects.auto_agent AND u.project = projects.id)
  WHERE EXISTS (SELECT 1 FROM agent_uses u WHERE u.agent = projects.auto_agent AND u.project = projects.id AND u.copy IS NOT NULL);
UPDATE projects SET auto_default = (SELECT u.copy FROM agent_uses u WHERE u.agent = projects.auto_default AND u.project = projects.id)
  WHERE EXISTS (SELECT 1 FROM agent_uses u WHERE u.agent = projects.auto_default AND u.project = projects.id AND u.copy IS NOT NULL);

DROP TABLE agents;
ALTER TABLE agents_new RENAME TO agents;
DROP TABLE agent_uses;
DROP TABLE agent_homes;
