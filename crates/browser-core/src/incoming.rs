use std::time::{Duration, Instant};

use engine::nav_policy::{self, Decision, Source};
use url::Url;

pub const MAX_INCOMING: usize = 10;
pub const INCOMING_WINDOW: Duration = Duration::from_secs(10);
pub const LOCALHOST_FROM_APPS: &str = "localhost_from_apps";

pub fn admit(seen: &mut Vec<Instant>, now: Instant, n: usize) -> usize {
    seen.retain(|at| now.duration_since(*at) < INCOMING_WINDOW);
    let admitted = n.min(MAX_INCOMING.saturating_sub(seen.len()));
    seen.extend(std::iter::repeat_n(now, admitted));
    admitted
}

pub fn admit_links(seen: &mut Vec<Instant>, now: Instant, mut urls: Vec<String>) -> (Vec<String>, usize) {
    let admitted = admit(seen, now, urls.len());
    let dropped = urls.len() - admitted;
    urls.truncate(admitted);
    (urls, dropped)
}

pub fn dropped_notice(dropped: usize) -> String {
    format!("{dropped} more {} from another app not opened", if dropped == 1 { "link" } else { "links" })
}

pub fn app_link(arg: &str, localhost_allowed: bool) -> Option<(Url, bool)> {
    let url = Url::parse(arg).ok()?;
    if nav_policy::decide(&url, Source::ExternalApp) != Decision::Allow {
        return None;
    }
    let typed = nav_policy::app_link_typed(&url, localhost_allowed);
    Some((url, typed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flood_of_links_is_capped_per_window_of_time() {
        let (mut seen, now) = (Vec::new(), Instant::now());
        assert_eq!(admit(&mut seen, now, 4), 4);
        assert_eq!(admit(&mut seen, now, 20), 6);
        assert_eq!(admit(&mut seen, now + Duration::from_secs(5), 1), 0);
        assert_eq!(admit(&mut seen, now + Duration::from_secs(11), 3), 3);
        let urls = |n: usize| (0..n).map(|i| format!("https://{i}.test/")).collect::<Vec<_>>();
        let (mut seen, now) = (Vec::new(), Instant::now());
        assert_eq!(admit_links(&mut seen, now, urls(12)), (urls(10), 2));
        assert_eq!(admit_links(&mut seen, now, urls(1)), (Vec::new(), 1));
    }

    #[test]
    fn links_from_other_apps_pass_the_policy_and_only_loopback_loads_as_typed() {
        let link = |arg: &str, allowed: bool| app_link(arg, allowed).map(|(u, typed)| (u.to_string(), typed));
        assert_eq!(link("https://example.com/a", true), Some(("https://example.com/a".into(), false)));
        assert_eq!(link("http://localhost:3000/", true), Some(("http://localhost:3000/".into(), true)));
        assert_eq!(link("http://127.0.0.1:8080/", true), Some(("http://127.0.0.1:8080/".into(), true)));
        assert_eq!(link("http://localhost:3000/", false), Some(("http://localhost:3000/".into(), false)));
        assert_eq!(link("http://192.168.1.1/", true), Some(("http://192.168.1.1/".into(), false)));
        for refused in ["file:///etc/passwd", "javascript:alert(1)", "mailto:a@example.com", "data:text/html,x", "--new-window", "not a url", ""] {
            assert_eq!(link(refused, true), None, "{refused}");
        }
    }

    #[test]
    fn the_dropped_notice_counts_links() {
        assert_eq!(dropped_notice(1), "1 more link from another app not opened");
        assert_eq!(dropped_notice(3), "3 more links from another app not opened");
    }
}
