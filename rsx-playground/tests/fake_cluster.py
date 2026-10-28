# pyright: basic
"""A cluster in memory for tests of the playground's glue.

It stands in for `rsx-cli cluster`: `up`, `down`, `restart`, `reset`, `ps`,
`plan` and `scenarios` of `cluster_cli`, with the same results and errors.
`crash(name)` leaves an exited record, as a killed daemon does.
"""

import itertools

import cluster_cli
import rsx_cli

PLAN = {
    "minimal": ["gw-0", "marketdata", "mark", "me-pengu", "recorder",
                "risk-0", "maker"],
    "duo": ["gw-0", "marketdata", "mark", "me-pengu", "me-sol",
            "recorder", "risk-0", "maker"],
}
SCENARIOS = [
    cluster_cli.Scenario("minimal", 1, "none", None),
    cluster_cli.Scenario("duo", 2, "none", None),
    cluster_cli.Scenario(
        "stress-low", 1, "none",
        {"rate": 100, "duration": 60, "type": "mixed"}),
]


class FakeCluster:
    def __init__(self):
        self.scenario = None
        self.names = []
        self.running = {}
        self.exited = set()
        self.calls = []
        self.error = None
        self.pids = itertools.count(40000)

    def started(self, *names):
        for name in names:
            self.running[name] = next(self.pids)
            self.exited.discard(name)

    def crash(self, name):
        self.running.pop(name)
        self.exited.add(name)

    def fail(self, message, code=1):
        self.error = rsx_cli.CliError(message, code)

    def check(self, call):
        self.calls.append(call)
        if self.error is not None:
            error, self.error = self.error, None
            raise error

    def row(self, name):
        if name in self.running:
            return cluster_cli.Row(
                name, self.running[name], "running", 1, f"log/{name}.log")
        state = "exited" if name in self.exited else "absent"
        pid = 1 if state == "exited" else None
        return cluster_cli.Row(name, pid, state, None, f"log/{name}.log")

    def report(self, started=(), stopped=(), cleared=(), skipped=()):
        return cluster_cli.Report(
            list(started), list(stopped), list(skipped), list(cleared))

    async def ps(self):
        self.calls.append(("ps",))
        names = list(dict.fromkeys(self.names + list(self.running)))
        return cluster_cli.View(self.scenario, [self.row(n) for n in names])

    async def plan(self, scenario):
        self.calls.append(("plan", scenario))
        if scenario not in PLAN:
            raise rsx_cli.CliError(f"unknown scenario {scenario!r}", 2)
        return [
            cluster_cli.PlanProcess(
                name, f"./target/debug/rsx-{name.split('-')[0]}",
                {"RSX_GW_HEALTH_ADDR": "127.0.0.1:9", "RSX_ME_SYMBOL_ID": "10",
                 "RSX_ME_HEALTH_ADDR": "127.0.0.1:9"})
            for name in PLAN[scenario]
        ]

    async def scenarios(self):
        self.calls.append(("scenarios",))
        return SCENARIOS

    async def up(self, scenario, only=(), build=False):
        self.check(("up", scenario, tuple(only), build))
        names = PLAN.get(scenario)
        if names is None:
            raise rsx_cli.CliError(f"unknown scenario {scenario!r}", 2)
        for name in only:
            if name not in names:
                raise rsx_cli.CliError(
                    f"{name!r} is not in scenario {scenario}", 2)
        if not only or self.scenario is None:
            self.scenario = scenario
            self.names = list(names)
        started = []
        skipped = []
        for name in only or names:
            if name in self.running:
                skipped.append(
                    cluster_cli.Skipped(name, "already running"))
            else:
                self.started(name)
                started.append(name)
        return self.report(started=started, skipped=skipped)

    async def down(self, only=()):
        self.calls.append(("down", tuple(only)))
        stopped = []
        skipped = []
        for name in only or list(self.running) + sorted(self.exited):
            if name in self.running:
                del self.running[name]
                stopped.append(name)
            elif name in self.exited:
                self.exited.discard(name)
                skipped.append(cluster_cli.Skipped(
                    name, "already exited; record removed"))
            else:
                skipped.append(cluster_cli.Skipped(
                    name, "no record: not started by the cluster"))
        if not only:
            self.scenario = None
            self.names = []
        return self.report(stopped=stopped, skipped=skipped)

    async def restart(self, name, scenario):
        self.check(("restart", name, scenario))
        if name not in PLAN.get(scenario, []):
            raise rsx_cli.CliError(
                f"{name!r} is not in scenario {scenario}", 2)
        stopped = [name] if name in self.running else []
        self.running.pop(name, None)
        self.started(name)
        return self.report(started=[name], stopped=stopped)

    async def reset(self):
        self.check(("reset",))
        report = await self.down()
        return self.report(
            stopped=report.stopped, cleared=["risk-state", "wal", "tips"])
