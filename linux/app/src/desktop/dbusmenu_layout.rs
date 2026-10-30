//! The top-bar icon's menu as `com.canonical.dbusmenu` describes it (shell-design §16.2) — pure:
//! the layout tree, each item's properties and what a click on an id means, with no bus. The Mac's
//! three items (`tray_rules::TrayItem`: Open Family Connect · Settings… · — · Quit Family Connect)
//! under a root, ids fixed for the process: the menu never changes while the app runs (the
//! language is read once at start-up, §23), so the layout's revision never moves.
//!
//! **Labels are escaped**: dbusmenu reads `_` as the access-key marker and `__` as one underscore,
//! so a translation with an underscore in it would lose it ([`escape_label`]).

use fc_core::text::string_catalog::StringCatalog;
use fc_logic::tray_rules::TrayItem;
use glib::prelude::*;

/// The root's id (the protocol's).
pub const ROOT_ID: i32 = 0;
/// The layout's revision: the menu never changes.
pub const REVISION: u32 = 1;

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item {
        id: i32,
        item: TrayItem,
        label: String,
    },
    Separator {
        id: i32,
    },
}

impl Entry {
    pub fn id(&self) -> i32 {
        match self {
            Entry::Item { id, .. } | Entry::Separator { id } => *id,
        }
    }

    /// Every property of the row, in the protocol's spelling.
    fn properties(&self) -> Vec<(&'static str, glib::Variant)> {
        match self {
            Entry::Item { label, .. } => vec![
                ("label", escape_label(label).to_variant()),
                ("enabled", true.to_variant()),
                ("visible", true.to_variant()),
            ],
            Entry::Separator { .. } => vec![
                ("type", "separator".to_variant()),
                ("visible", true.to_variant()),
            ],
        }
    }
}

/// What a menu row does when it is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chosen {
    /// `gio::Application::activate` — the main window comes forward.
    Activate,
    /// An `app.` action (without its prefix).
    Action(&'static str),
}

/// The action of an item (seams/desktop.rs: "Open Family Connect" activates the application,
/// "Settings…" is `app.show-settings`, "Quit Family Connect" `app.quit`).
pub fn chosen(item: TrayItem) -> Chosen {
    match item {
        TrayItem::Open => Chosen::Activate,
        TrayItem::Settings => Chosen::Action(crate::seams::actions::SHOW_SETTINGS.name),
        TrayItem::Quit => Chosen::Action(crate::seams::actions::QUIT.name),
    }
}

/// A label as dbusmenu shows it literally: every `_` doubled.
pub fn escape_label(label: &str) -> String {
    label.replace('_', "__")
}

/// Whether an `Event`'s id is a click (the protocol's other events — `hovered`, `opened`,
/// `closed` — do nothing here).
pub fn is_click(event_id: &str) -> bool {
    event_id == "clicked"
}

/// The whole menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuLayout {
    entries: Vec<Entry>,
}

impl MenuLayout {
    /// The Mac's three items in the reader's language, a separator where `TrayItem::separated`
    /// says. Ids count from 1 in order.
    pub fn new(say: &dyn StringCatalog) -> MenuLayout {
        let mut entries = Vec::new();
        let mut next = ROOT_ID;
        for item in TrayItem::ALL {
            if item.separated() {
                next += 1;
                entries.push(Entry::Separator { id: next });
            }
            next += 1;
            entries.push(Entry::Item {
                id: next,
                item,
                label: item.label(say),
            });
        }
        MenuLayout { entries }
    }

    /// The item behind `id` (none for the root, a separator or an unknown id).
    pub fn item_for(&self, id: i32) -> Option<TrayItem> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Item { id: at, item, .. } if *at == id => Some(*item),
            _ => None,
        })
    }

    /// The id of `item`.
    #[cfg(test)]
    pub fn id_of(&self, item: TrayItem) -> Option<i32> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Item { id, item: at, .. } if *at == item => Some(*id),
            _ => None,
        })
    }

    /// Whether `id` names a node of this menu (the root included).
    pub fn contains(&self, id: i32) -> bool {
        id == ROOT_ID || self.entries.iter().any(|entry| entry.id() == id)
    }

    /// The properties of node `id`, only those in `names` (all when `names` is empty); `None` for
    /// an unknown id.
    pub fn properties(
        &self,
        id: i32,
        names: &[String],
    ) -> Option<Vec<(&'static str, glib::Variant)>> {
        let all = if id == ROOT_ID {
            vec![("children-display", "submenu".to_variant())]
        } else {
            self.entries.iter().find(|e| e.id() == id)?.properties()
        };
        Some(
            all.into_iter()
                .filter(|(name, _)| names.is_empty() || names.iter().any(|n| n == name))
                .collect(),
        )
    }

    /// One property of node `id`.
    pub fn property(&self, id: i32, name: &str) -> Option<glib::Variant> {
        self.properties(id, &[name.to_string()])?
            .into_iter()
            .next()
            .map(|(_, value)| value)
    }

    /// `GetLayout`'s answer, `(u(ia{sv}av))`: the subtree under `parent` down `depth` levels (−1:
    /// all; 0: the node alone), with the properties in `names`. `None` for an unknown parent.
    pub fn layout(&self, parent: i32, depth: i32, names: &[String]) -> Option<glib::Variant> {
        let node = self.node(parent, depth, names)?;
        Some(glib::Variant::tuple_from_iter([
            REVISION.to_variant(),
            node,
        ]))
    }

    /// `GetGroupProperties`' answer, `a(ia{sv})`: every known id of `ids` (all nodes when `ids` is
    /// empty) with the properties in `names`.
    pub fn group_properties(&self, ids: &[i32], names: &[String]) -> glib::Variant {
        let wanted: Vec<i32> = if ids.is_empty() {
            std::iter::once(ROOT_ID)
                .chain(self.entries.iter().map(Entry::id))
                .collect()
        } else {
            ids.to_vec()
        };
        let rows: Vec<glib::Variant> = wanted
            .into_iter()
            .filter_map(|id| {
                let properties = self.properties(id, names)?;
                Some(glib::Variant::tuple_from_iter([
                    id.to_variant(),
                    super::dbus::vardict(&properties),
                ]))
            })
            .collect();
        glib::Variant::array_from_iter_with_type(
            glib::VariantTy::new("(ia{sv})").expect("a valid type"),
            rows,
        )
    }

    /// One node, `(ia{sv}av)`.
    fn node(&self, id: i32, depth: i32, names: &[String]) -> Option<glib::Variant> {
        let properties = self.properties(id, names)?;
        let children: Vec<glib::Variant> = if id == ROOT_ID && depth != 0 {
            self.entries
                .iter()
                .filter_map(|entry| self.node(entry.id(), depth - 1, names))
                .map(|child| glib::Variant::from_variant(&child))
                .collect()
        } else {
            Vec::new()
        };
        Some(glib::Variant::tuple_from_iter([
            id.to_variant(),
            super::dbus::vardict(&properties),
            glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, children),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;

    fn layout() -> MenuLayout {
        MenuLayout::new(&EnglishCatalog::new())
    }

    /// `(ia{sv}av)` → (id, label or type, children).
    fn read(node: &glib::Variant) -> (i32, String, Vec<glib::Variant>) {
        let id = node.child_value(0).get::<i32>().unwrap();
        let properties = glib::VariantDict::new(Some(&node.child_value(1)));
        let text = properties
            .lookup_value("label", None)
            .or_else(|| properties.lookup_value("type", None))
            .and_then(|v| v.str().map(str::to_string))
            .unwrap_or_default();
        let children = node
            .child_value(2)
            .iter()
            .map(|child| child.as_variant().unwrap())
            .collect();
        (id, text, children)
    }

    #[test]
    fn the_menu_is_the_macs_three_items_with_a_separator_before_quit() {
        let menu = layout();
        let answer = menu.layout(ROOT_ID, -1, &[]).unwrap();
        assert_eq!(answer.type_().as_str(), "(u(ia{sv}av))");
        assert_eq!(answer.child_value(0).get::<u32>(), Some(REVISION));
        let (id, _, children) = read(&answer.child_value(1));
        assert_eq!(id, ROOT_ID);
        let rows: Vec<(i32, String)> = children
            .iter()
            .map(|child| {
                let (id, text, grandchildren) = read(child);
                assert!(grandchildren.is_empty());
                (id, text)
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (1, "Open Family Connect".to_string()),
                (2, "Settings…".to_string()),
                (3, "separator".to_string()),
                (4, "Quit Family Connect".to_string()),
            ]
        );
        let root = glib::VariantDict::new(Some(&answer.child_value(1).child_value(1)));
        assert_eq!(
            root.lookup_value("children-display", None)
                .and_then(|v| v.str().map(str::to_string))
                .as_deref(),
            Some("submenu")
        );
    }

    #[test]
    fn clicks_map_to_the_seams_actions() {
        let menu = layout();
        assert_eq!(menu.item_for(1), Some(TrayItem::Open));
        assert_eq!(menu.item_for(2), Some(TrayItem::Settings));
        assert_eq!(menu.item_for(3), None);
        assert_eq!(menu.item_for(4), Some(TrayItem::Quit));
        assert_eq!(menu.item_for(0), None);
        assert_eq!(menu.item_for(99), None);
        assert_eq!(chosen(TrayItem::Open), Chosen::Activate);
        assert_eq!(chosen(TrayItem::Settings), Chosen::Action("show-settings"));
        assert_eq!(chosen(TrayItem::Quit), Chosen::Action("quit"));
        assert_eq!(menu.id_of(TrayItem::Quit), Some(4));
        assert!(is_click("clicked"));
        assert!(!is_click("hovered"));
    }

    #[test]
    fn depth_and_property_names_limit_the_answer() {
        let menu = layout();
        let (_, _, children) = read(&menu.layout(ROOT_ID, 0, &[]).unwrap().child_value(1));
        assert!(children.is_empty());
        let only_labels = menu.layout(ROOT_ID, 1, &["label".to_string()]).unwrap();
        let (_, _, children) = read(&only_labels.child_value(1));
        let separator = glib::VariantDict::new(Some(&children[2].child_value(1)));
        assert!(!separator.contains("type"));
        let (id, text, _) = read(&menu.layout(2, -1, &[]).unwrap().child_value(1));
        assert_eq!((id, text.as_str()), (2, "Settings…"));
        assert!(menu.layout(42, -1, &[]).is_none());
        assert_eq!(
            menu.property(4, "label")
                .and_then(|v| v.str().map(str::to_string))
                .as_deref(),
            Some("Quit Family Connect")
        );
        assert!(menu.property(4, "icon-name").is_none());
        assert!(menu.contains(0) && menu.contains(3) && !menu.contains(5));
    }

    #[test]
    fn group_properties_answer_known_ids_only() {
        let menu = layout();
        let all = menu.group_properties(&[], &[]);
        assert_eq!(all.type_().as_str(), "a(ia{sv})");
        assert_eq!(all.n_children(), 5);
        let some = menu.group_properties(&[1, 77, 4], &["label".to_string()]);
        let ids: Vec<i32> = some
            .iter()
            .map(|row| row.child_value(0).get::<i32>().unwrap())
            .collect();
        assert_eq!(ids, vec![1, 4]);
    }

    #[test]
    fn an_underscore_in_a_label_is_shown_not_taken_as_an_access_key() {
        assert_eq!(escape_label("Open_Family"), "Open__Family");
        assert_eq!(escape_label("Settings…"), "Settings…");
    }
}
