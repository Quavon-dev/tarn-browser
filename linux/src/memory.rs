use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::path::Path;
use std::time::{Duration, Instant};

use browser_core::settings;
use browser_core::tabs::TabKey;
use store::TabKind;

use crate::browser::Browser;

const WINDOW: Duration = Duration::from_secs(60);
const MB: f64 = (1u64 << 20) as f64;
const TICK_NS: u64 = 10_000_000;
const WEB_PROCESS: &str = "WebKitWebProces";
const HEAVY_TABS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub footprint: u64,
    pub cpu_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cost {
    pub footprint: u64,
    pub cpu: Option<f64>,
    pub sharing: usize,
}

#[derive(Default)]
pub struct Samples(HashMap<i32, VecDeque<(Instant, Usage)>>);

impl Samples {
    pub fn record(&mut self, live: HashMap<i32, Usage>, at: Instant) {
        self.0.retain(|pid, _| live.contains_key(pid));
        for (pid, usage) in live {
            let samples = self.0.entry(pid).or_default();
            samples.push_back((at, usage));
            while samples.len() > 2 && at.duration_since(samples[1].0) >= WINDOW {
                samples.pop_front();
            }
        }
    }

    fn latest(&self, pid: i32) -> Option<Usage> {
        self.0.get(&pid)?.back().map(|(_, u)| *u)
    }

    fn cpu(&self, pid: i32) -> Option<f64> {
        let samples = self.0.get(&pid)?;
        let ((t0, u0), (t1, u1)) = (samples.front()?, samples.back()?);
        let elapsed = t1.duration_since(*t0).as_nanos();
        (elapsed > 0).then(|| u1.cpu_ns.saturating_sub(u0.cpu_ns) as f64 * 100. / elapsed as f64)
    }
}

pub fn costs<K: Copy + Eq + Hash>(tabs: &[(K, i32)], samples: &Samples) -> HashMap<K, Cost> {
    let mut sharing: HashMap<i32, usize> = HashMap::new();
    for (_, pid) in tabs {
        *sharing.entry(*pid).or_default() += 1;
    }
    tabs.iter()
        .filter_map(|(key, pid)| Some((*key, Cost { footprint: samples.latest(*pid)?.footprint, cpu: samples.cpu(*pid), sharing: sharing[pid] })))
        .collect()
}

pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / MB;
    if mb < 1000. { format!("{mb:.0} MB") } else { format!("{:.1} GB", mb / 1024.) }
}

pub fn detail(cost: Option<Cost>, asleep: bool) -> String {
    let Some(cost) = cost.filter(|_| !asleep) else { return if asleep { "Sleeping" } else { "Not measured" }.into() };
    let mut parts = vec![size(cost.footprint)];
    parts.extend(cost.cpu.map(|cpu| format!("{cpu:.1}% CPU")));
    if cost.sharing > 1 {
        parts.push(format!("Process shared by {} tabs", cost.sharing));
    }
    parts.join(" · ")
}

fn stat_fields(stat: &str) -> Option<Vec<&str>> {
    let (_, rest) = stat.rsplit_once(')')?;
    Some(rest.split_whitespace().collect())
}

pub fn parse_usage(stat: &str, status: &str) -> Option<Usage> {
    let fields = stat_fields(stat)?;
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    let kb = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.split_whitespace().next()?.parse::<u64>().ok()?;
    Some(Usage { footprint: kb * 1024, cpu_ns: ticks * TICK_NS })
}

pub fn usage(root: &Path, pid: i32, ancestor: i32) -> Option<Usage> {
    let dir = root.join(pid.to_string());
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    let (mut parent, name) = parent_and_name(&stat)?;
    if name != WEB_PROCESS {
        return None;
    }
    for _ in 0..64 {
        if parent == ancestor {
            return parse_usage(&stat, &std::fs::read_to_string(dir.join("status")).ok()?);
        }
        if parent <= 1 {
            return None;
        }
        parent = parent_and_name(&std::fs::read_to_string(root.join(parent.to_string()).join("stat")).ok()?)?.0;
    }
    None
}

pub fn pressure(meminfo: &str) -> bool {
    let field = |name: &str| meminfo.lines().find_map(|l| l.strip_prefix(name)?.split_whitespace().next()?.parse::<u64>().ok());
    field("MemAvailable:").zip(field("MemTotal:")).is_some_and(|(available, total)| available * 10 < total)
}

fn parent_and_name(stat: &str) -> Option<(i32, &str)> {
    let name = stat.split_once('(')?.1.rsplit_once(')')?.0;
    Some((stat_fields(stat)?.get(1)?.parse().ok()?, name))
}

pub fn web_processes(root: &Path, ancestor: i32) -> HashSet<i32> {
    let mut parents = HashMap::new();
    let mut web = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        let Some((parent, name)) = parent_and_name(&stat) else { continue };
        parents.insert(pid, parent);
        if name == WEB_PROCESS {
            web.push(pid);
        }
    }
    let descends = |mut pid: i32| {
        for _ in 0..64 {
            match parents.get(&pid) {
                Some(&parent) if parent == ancestor => return true,
                Some(&parent) if parent > 1 => pid = parent,
                _ => return false,
            }
        }
        false
    };
    web.into_iter().filter(|pid| descends(*pid)).collect()
}

impl Browser {
    pub fn sample_processes(&mut self) {
        let root = Path::new("/proc");
        let live = self.processes.values().filter_map(|pid| Some((*pid, usage(root, *pid, std::process::id() as i32)?))).collect();
        self.samples.record(live, Instant::now());
    }

    fn tab_processes(&self) -> Vec<(TabKey, i32)> {
        let live: HashSet<TabKey> = self.model.lifecycle.live().collect();
        self.processes.iter().filter(|(k, _)| live.contains(k)).map(|(k, p)| (*k, *p)).collect()
    }

    pub fn tab_costs(&self) -> HashMap<TabKey, Cost> {
        costs(&self.tab_processes(), &self.samples)
    }

    pub fn heavy_tabs(&self) -> HashSet<TabKey> {
        let costs = self.tab_costs();
        let idle = |k: &TabKey| self.used.get(k).map_or(Duration::MAX, Instant::elapsed);
        let awake = self
            .model
            .lifecycle
            .live()
            .filter(|k| Some(*k) != self.model.active)
            .map(|k| (k, costs.get(&k).map_or(0, |c| c.footprint), self.playing.contains(&k), idle(&k)));
        engine::lifecycle::heaviest(awake, HEAVY_TABS).into_iter().filter(|k| costs.contains_key(k)).collect()
    }

    pub fn sleep_tab(&mut self, key: TabKey) {
        if self.model.active == Some(key) {
            return;
        }
        self.model.lifecycle.remove(key);
        self.unload(key);
    }

    pub fn unload(&mut self, key: TabKey) {
        if let Some(page) = self.pages.get_mut(&key) {
            (page.load, page.loading) = (None, false);
        }
        self.processes.remove(&key);
        self.playing.remove(&key);
    }

    pub fn forget_silent_views(&mut self) {
        let shown: HashSet<TabKey> = self.model.lifecycle.live().filter(|k| self.model.tab(*k).is_some()).collect();
        self.playing.retain(|k| shown.contains(k));
    }

    pub fn idle_tick(&mut self, windows: usize) {
        let short = std::fs::read_to_string("/proc/meminfo").is_ok_and(|m| pressure(&m));
        let now = Instant::now();
        self.sleep_idle_tabs(now, short);
        if settings::sleep_after(&self.core.disk).is_some() {
            self.sleep_over_budget(now, engine::lifecycle::BACKGROUND_BUDGET / windows.max(1) as u64);
        }
    }

    pub fn sleep_over_budget(&mut self, now: Instant, budget: u64) {
        let costs = self.tab_costs();
        let awake: Vec<_> = self
            .model
            .lifecycle
            .live()
            .filter(|k| Some(*k) != self.model.active && !self.in_split(*k))
            .map(|k| {
                let pinned = self.model.tab(k).is_some_and(|t| t.kind != TabKind::Ephemeral);
                let idle = self.used.get(&k).map_or(Duration::ZERO, |used| now.saturating_duration_since(*used));
                let share = costs.get(&k).map_or(0, |c| c.footprint / c.sharing as u64);
                (k, share, self.playing.contains(&k), engine::lifecycle::budget_idle(idle, pinned))
            })
            .collect();
        for key in engine::lifecycle::over_budget(awake, budget) {
            self.sleep_tab(key);
        }
    }

    pub fn set_live_tabs(&mut self, ix: usize) {
        if let Some(name) = settings::LIVE_TABS.get(ix) {
            self.core.disk.set_setting(settings::LIVE_TABS_KEY, name).ok();
        }
    }

    /// Whether this turned sleeping on after Never, so idle times start afresh.
    pub fn set_sleep_after(&mut self, ix: usize) -> bool {
        let was_never = settings::sleep_after(&self.core.disk).is_none();
        let Some((name, _)) = settings::SLEEP_AFTER.get(ix) else { return false };
        self.core.disk.set_setting(settings::SLEEP_AFTER_KEY, name).is_ok() && was_never && settings::sleep_after(&self.core.disk).is_some()
    }

    pub fn apply_tab_choices(&mut self, restart: bool) {
        let shown = self.model.active.map(|active| self.split_partners(active).into_iter().chain([active]).collect::<Vec<_>>()).unwrap_or_default();
        let playing = &self.playing;
        for key in self.model.lifecycle.set_max_live(settings::live_tabs(&self.core.disk), |k| playing.contains(&k) || shown.contains(&k)) {
            self.unload(key);
        }
        if restart {
            let now = Instant::now();
            self.used.values_mut().for_each(|used| *used = now);
        }
        self.idle_tick();
    }

    pub fn sleep_idle_tabs(&mut self, now: Instant, pressure: bool) {
        let max_idle = settings::max_idle(settings::sleep_after(&self.core.disk), pressure);
        self.forget_silent_views();
        let live: Vec<TabKey> = self.model.lifecycle.live().collect();
        for key in live {
            let Some(tab) = self.model.tab(key) else {
                self.used.insert(key, now);
                continue;
            };
            let ephemeral = tab.kind == TabKind::Ephemeral;
            let used = *self.used.entry(key).or_insert(now);
            if ephemeral
                && self.model.active != Some(key)
                && !self.playing.contains(&key)
                && max_idle.is_some_and(|max| now.saturating_duration_since(used) >= max)
            {
                self.sleep_tab(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_of(footprint: u64, cpu_ns: u64) -> Usage {
        Usage { footprint, cpu_ns }
    }

    #[test]
    fn proc_files_parse_into_memory_and_cpu_time() {
        let stat = "4242 (WebKitWebProces) S 4200 4242 4200 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 30 0";
        let status = "Name:\tWebKitWebProcess\nVmRSS:\t  204800 kB\nThreads:\t30\n";
        assert_eq!(parse_usage(stat, status), Some(usage_of(204_800 * 1024, 300 * TICK_NS)));
        assert_eq!(parent_and_name(stat), Some((4200, "WebKitWebProces")));
        assert_eq!(parent_and_name("7 (a) b)) S 1 7"), Some((1, "a) b)")));
        assert_eq!(parse_usage("garbage", status), None);
    }

    #[test]
    fn web_processes_are_the_named_descendants_of_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let proc = |pid: i32, name: &str, parent: i32| {
            let d = dir.path().join(pid.to_string());
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("stat"), format!("{pid} ({name}) S {parent} 0 0 0 0 0 0 0 0 0 1 1")).unwrap();
        };
        proc(10, "tarn", 1);
        proc(11, "bwrap", 10);
        proc(12, WEB_PROCESS, 11);
        proc(13, WEB_PROCESS, 10);
        proc(20, WEB_PROCESS, 1);
        assert_eq!(web_processes(dir.path(), 10), HashSet::from([12, 13]));
        let status = "VmRSS:\t1 kB\n";
        for pid in [11, 12, 20] {
            std::fs::write(dir.path().join(pid.to_string()).join("status"), status).unwrap();
        }
        assert!(usage(dir.path(), 12, 10).is_some());
        assert_eq!(usage(dir.path(), 20, 10), None, "a web process of another app");
        assert_eq!(usage(dir.path(), 11, 10), None, "a reused pid that is no web process");
    }

    #[test]
    fn cpu_is_measured_over_the_last_minute_and_tabs_share_a_process() {
        let (mut s, t) = (Samples::default(), Instant::now());
        s.record(HashMap::from([(7, usage_of(100, 0))]), t);
        assert_eq!(s.cpu(7), None);
        s.record(HashMap::from([(7, usage_of(100, 1_000_000_000))]), t + Duration::from_secs(10));
        assert_eq!(s.cpu(7), Some(10.));
        let costs = costs(&[('a', 7), ('b', 7), ('c', 8)], &s);
        assert_eq!(costs[&'a'], Cost { footprint: 100, cpu: Some(10.), sharing: 2 });
        assert!(!costs.contains_key(&'c'));
        assert_eq!(detail(Some(costs[&'a']), false), "0 MB · 10.0% CPU · Process shared by 2 tabs");
        assert_eq!(detail(None, true), "Sleeping");
        assert_eq!(size(3 << 30), "3.0 GB");
    }

    #[test]
    fn memory_is_short_below_a_tenth_available() {
        assert!(pressure("MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:    1500000 kB\n"));
        assert!(!pressure("MemTotal:       16000000 kB\nMemAvailable:    1700000 kB\n"));
        assert!(!pressure("MemTotal: 16000000 kB\n"), "an old kernel without MemAvailable");
    }
}
