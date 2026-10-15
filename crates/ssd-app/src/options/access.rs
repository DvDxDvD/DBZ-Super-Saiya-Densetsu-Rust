//! The Accessibility page's choices as the HD drawing uses them (`access.*` of the PC file): the style the
//! menus are shown in. The menus' own draw lists are styled by the driver; this styles what the app adds over
//! the field (the cheats' notice, the badges of the save-slot list).

use ssd_menus::options::{Values, accessibility};
use ssd_menus::{DrawList, Style};

/// The style of the PC file's values.
#[must_use]
pub fn style(values: &Values) -> Style {
    accessibility::style(values)
}

/// `list` shown in the style of the PC file's values.
#[must_use]
pub fn restyle(values: &Values, list: DrawList) -> DrawList {
    style(values).apply(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::Op;

    #[test]
    fn a_notice_is_restyled_only_when_a_choice_is_on() {
        let notice = ssd_menus::screens::notice("Cheats off for this fight");
        assert_eq!(restyle(&Values::new(), notice.clone()), notice);
        let mut values = Values::new();
        values.set("access.contrast", 1);
        let shown = restyle(&values, notice);
        assert!(shown.ops().iter().any(|op| matches!(op, Op::WindowHc { .. })));
    }
}
