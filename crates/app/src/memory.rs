//! What each tab's web content process costs, measured per process: WebKit may run several tabs
//! in one process, and then each of them shows that process's total.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

use platform::memory::Usage;

use crate::browser::{Browser, TabKey};

const WINDOW: Duration = Duration::from_secs(60);
const MB: f64 = (1u64 << 20) as f64;

/// Each process's recent samples, oldest first; one at or before the start of the window stays as
/// the baseline.
#[derive(Default)]
pub(crate) struct Samples(HashMap<i32, VecDeque<(Instant, Usage)>>);

impl Samples {
    /// Adds one sample per live process and forgets the processes not among them.
    fn record(&mut self, live: HashMap<i32, Usage>, at: Instant) {
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

    /// Percent of one core over the last minute, or since the first sample when that is younger.
    fn cpu(&self, pid: i32) -> Option<f64> {
        let samples = self.0.get(&pid)?;
        let ((t0, u0), (t1, u1)) = (samples.front()?, samples.back()?);
        let elapsed = t1.duration_since(*t0).as_nanos();
        (elapsed > 0).then(|| u1.cpu_ns.saturating_sub(u0.cpu_ns) as f64 * 100. / elapsed as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cost {
    pub footprint: u64,
    pub cpu: Option<f64>,
    /// How many of the tabs run in the same process.
    pub sharing: usize,
}

/// Each tab's cost from its process's latest sample.
fn costs<K: Copy + Eq + Hash>(tabs: &[(K, i32)], samples: &Samples) -> HashMap<K, Cost> {
    let mut sharing: HashMap<i32, usize> = HashMap::new();
    for (_, pid) in tabs {
        *sharing.entry(*pid).or_default() += 1;
    }
    tabs.iter()
        .filter_map(|(key, pid)| Some((*key, Cost { footprint: samples.latest(*pid)?.footprint, cpu: samples.cpu(*pid), sharing: sharing[pid] })))
        .collect()
}

/// "312 MB", "1.4 GB".
pub(crate) fn size(bytes: u64) -> String {
    let mb = bytes as f64 / MB;
    if mb < 1000. { format!("{mb:.0} MB") } else { format!("{:.1} GB", mb / 1024.) }
}

/// The library's line under a tab.
pub(crate) fn detail(cost: Option<Cost>, asleep: bool) -> String {
    let Some(cost) = cost.filter(|_| !asleep) else { return if asleep { "Sleeping" } else { "Not measured" }.into() };
    let mut parts = vec![size(cost.footprint)];
    parts.extend(cost.cpu.map(|cpu| format!("{cpu:.1}% CPU")));
    if cost.sharing > 1 {
        parts.push(format!("Process shared by {} tabs", cost.sharing));
    }
    parts.join(" · ")
}

impl Browser {
    fn tab_processes(&self) -> Vec<(TabKey, i32)> {
        self.views.iter().filter_map(|(key, view)| Some((*key, view.process_id()?))).collect()
    }

    /// Samples every tab's process; the CPU figures come from the change between samples.
    pub(crate) fn sample_processes(&mut self) {
        let live = self.tab_processes().into_iter().filter_map(|(_, pid)| Some((pid, platform::memory::usage(pid)?))).collect();
        self.samples.record(live, Instant::now());
    }

    pub(crate) fn tab_costs(&self) -> HashMap<TabKey, Cost> {
        costs(&self.tab_processes(), &self.samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(footprint: u64, cpu_ns: u64) -> Usage {
        Usage { footprint, cpu_ns }
    }

    #[test]
    fn cpu_is_measured_over_the_last_minute() {
        let (mut s, t) = (Samples::default(), Instant::now());
        s.record(HashMap::from([(7, usage(1, 0))]), t);
        assert_eq!(s.cpu(7), None);
        s.record(HashMap::from([(7, usage(1, 1_000_000_000))]), t + Duration::from_secs(10));
        assert_eq!(s.cpu(7), Some(10.));
        s.record(HashMap::from([(7, usage(1, 1_000_000_000))]), t + Duration::from_secs(70));
        assert_eq!(s.0[&7].len(), 2, "the sample from a minute ago is the baseline");
        assert_eq!(s.cpu(7), Some(0.));
        s.record(HashMap::new(), t + Duration::from_secs(80));
        assert_eq!(s.cpu(7), None);
    }

    #[test]
    fn a_reused_process_id_does_not_count_backwards() {
        let (mut s, t) = (Samples::default(), Instant::now());
        s.record(HashMap::from([(7, usage(1, 5_000))]), t);
        s.record(HashMap::from([(7, usage(1, 0))]), t + Duration::from_secs(1));
        assert_eq!(s.cpu(7), Some(0.));
    }

    #[test]
    fn tabs_sharing_a_process_each_show_its_total() {
        let mut s = Samples::default();
        s.record(HashMap::from([(1, usage(100, 0)), (2, usage(50, 0))]), Instant::now());
        let costs = costs(&[('a', 1), ('b', 1), ('c', 2), ('d', 3)], &s);
        assert_eq!(costs[&'a'], Cost { footprint: 100, cpu: None, sharing: 2 });
        assert_eq!(costs[&'b'], costs[&'a']);
        assert_eq!(costs[&'c'].sharing, 1);
        assert!(!costs.contains_key(&'d'), "a process without a sample is not measured");
    }

    #[test]
    fn details_read_like_activity_monitor() {
        assert_eq!(size(312 << 20), "312 MB");
        assert_eq!(size(1500 << 20), "1.5 GB");
        let cost = Cost { footprint: 200 << 20, cpu: Some(3.04), sharing: 1 };
        assert_eq!(detail(Some(cost), false), "200 MB · 3.0% CPU");
        assert_eq!(detail(Some(Cost { cpu: None, sharing: 3, ..cost }), false), "200 MB · Process shared by 3 tabs");
        assert_eq!(detail(None, false), "Not measured");
        assert_eq!(detail(Some(cost), true), "Sleeping");
    }
}
