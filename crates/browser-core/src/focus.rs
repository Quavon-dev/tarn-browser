/// The parts of a browser window F6 moves the keyboard between, in its order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Page,
    Address,
    Sidebar,
}

const ORDER: [Region; 3] = [Region::Page, Region::Address, Region::Sidebar];

/// Where F6 (Shift+F6 with `back`) moves the keyboard from `from`, skipping the page while no tab
/// shows one and the sidebar while it is hidden. The address field is always there.
pub fn next_region(from: Region, back: bool, page: bool, sidebar: bool) -> Region {
    let shown = |r: Region| match r {
        Region::Page => page,
        Region::Address => true,
        Region::Sidebar => sidebar,
    };
    let at = ORDER.iter().position(|r| *r == from).unwrap_or(0);
    (1..ORDER.len()).map(|step| ORDER[if back { at + ORDER.len() - step } else { at + step } % ORDER.len()]).find(|r| shown(*r)).unwrap_or(Region::Address)
}

#[cfg(test)]
mod tests {
    use super::{Region::*, next_region};

    #[test]
    fn f6_cycles_page_address_sidebar() {
        assert_eq!(next_region(Page, false, true, true), Address);
        assert_eq!(next_region(Address, false, true, true), Sidebar);
        assert_eq!(next_region(Sidebar, false, true, true), Page);
    }

    #[test]
    fn shift_f6_goes_back() {
        assert_eq!(next_region(Page, true, true, true), Sidebar);
        assert_eq!(next_region(Sidebar, true, true, true), Address);
        assert_eq!(next_region(Address, true, true, true), Page);
    }

    #[test]
    fn hidden_regions_are_skipped() {
        assert_eq!(next_region(Address, false, true, false), Page, "no sidebar");
        assert_eq!(next_region(Sidebar, false, false, true), Address, "no page");
        assert_eq!(next_region(Address, false, false, false), Address, "only the address field");
    }
}
