//! The top-bar icon's menu on the bus: `/MenuBar`, interface `com.canonical.dbusmenu` (version 3,
//! shell-design §16.2), exported on the app's own connection. Ubuntu's AppIndicator extension reads
//! it with `GetLayout` and sends `Event(id, "clicked", …)`; everything it answers comes from the
//! pure [`MenuLayout`](super::dbusmenu_layout::MenuLayout), and a click becomes the item's
//! [`Chosen`](super::dbusmenu_layout::Chosen) — the application activated, or an `app.` action.

use std::rc::Rc;

use gio::prelude::*;

use super::dbus::{self, Exported};
use super::dbusmenu_layout::{self, MenuLayout};
use fc_logic::tray_rules::TrayItem;

/// Where the menu lives (the item's `Menu` property).
pub const MENU_PATH: &str = "/MenuBar";
/// The interface.
pub const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
/// The protocol version the layout follows.
pub const MENU_VERSION: u32 = 3;

/// The interface's introspection (libdbusmenu's, the signals included).
pub const MENU_XML: &str = r#"<node>
  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
    <method name="GetLayout">
      <arg type="i" name="parentId" direction="in"/>
      <arg type="i" name="recursionDepth" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="u" name="revision" direction="out"/>
      <arg type="(ia{sv}av)" name="layout" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg type="ai" name="ids" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="a(ia{sv})" name="properties" direction="out"/>
    </method>
    <method name="GetProperty">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="name" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <method name="Event">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="eventId" direction="in"/>
      <arg type="v" name="data" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="EventGroup">
      <arg type="a(isvu)" name="events" direction="in"/>
      <arg type="ai" name="idErrors" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg type="i" name="id" direction="in"/>
      <arg type="b" name="needUpdate" direction="out"/>
    </method>
    <method name="AboutToShowGroup">
      <arg type="ai" name="ids" direction="in"/>
      <arg type="ai" name="updatesNeeded" direction="out"/>
      <arg type="ai" name="idErrors" direction="out"/>
    </method>
    <signal name="ItemsPropertiesUpdated">
      <arg type="a(ia{sv})" name="updatedProps" direction="out"/>
      <arg type="a(ias)" name="removedProps" direction="out"/>
    </signal>
    <signal name="LayoutUpdated">
      <arg type="u" name="revision" direction="out"/>
      <arg type="i" name="parent" direction="out"/>
    </signal>
    <signal name="ItemActivationRequested">
      <arg type="i" name="id" direction="out"/>
      <arg type="u" name="timestamp" direction="out"/>
    </signal>
  </interface>
</node>"#;

const INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";

/// Exports the menu on `connection` at [`MENU_PATH`]; `chosen(item)` runs (on an idle, after the
/// call has been answered) for every click. Unexported when the answer is dropped.
pub fn export(
    connection: &gio::DBusConnection,
    menu: Rc<MenuLayout>,
    chosen: Rc<dyn Fn(TrayItem)>,
) -> Result<Exported, glib::Error> {
    let info = dbus::interface(MENU_XML, MENU_INTERFACE);
    let mut exported = Exported::new(connection);
    let methods = Rc::clone(&menu);
    let id = connection
        .register_object(MENU_PATH, &info)
        .method_call(move |_, _, _, _, method, parameters, invocation| {
            answer(&methods, &chosen, method, &parameters, invocation);
        })
        .property(|_, _, _, _, name| property(name))
        .build()?;
    exported.push(id);
    Ok(exported)
}

/// The interface's properties.
fn property(name: &str) -> glib::Variant {
    match name {
        "Version" => MENU_VERSION.to_variant(),
        "TextDirection" => match gtk::Widget::default_direction() {
            gtk::TextDirection::Rtl => "rtl".to_variant(),
            _ => "ltr".to_variant(),
        },
        "Status" => "normal".to_variant(),
        // "IconThemePath"
        _ => Vec::<String>::new().to_variant(),
    }
}

fn answer(
    menu: &MenuLayout,
    chosen: &Rc<dyn Fn(TrayItem)>,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "GetLayout" => {
            let Some((parent, depth, names)) = parameters.get::<(i32, i32, Vec<String>)>() else {
                return invocation.return_dbus_error(INVALID_ARGS, "(iias) expected");
            };
            match menu.layout(parent, depth, &names) {
                Some(layout) => invocation.return_value(Some(&layout)),
                None => invocation.return_dbus_error(INVALID_ARGS, "unknown menu id"),
            }
        }
        "GetGroupProperties" => {
            let Some((ids, names)) = parameters.get::<(Vec<i32>, Vec<String>)>() else {
                return invocation.return_dbus_error(INVALID_ARGS, "(aias) expected");
            };
            let rows = menu.group_properties(&ids, &names);
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([rows])));
        }
        "GetProperty" => {
            let Some((id, name)) = parameters.get::<(i32, String)>() else {
                return invocation.return_dbus_error(INVALID_ARGS, "(is) expected");
            };
            match menu.property(id, &name) {
                Some(value) => invocation.return_value(Some(&glib::Variant::tuple_from_iter([
                    glib::Variant::from_variant(&value),
                ]))),
                None => invocation.return_dbus_error(INVALID_ARGS, "unknown menu id or property"),
            }
        }
        "Event" => {
            let id = parameters.child_value(0).get::<i32>().unwrap_or(-1);
            let event = parameters
                .child_value(1)
                .str()
                .unwrap_or_default()
                .to_string();
            if !menu.contains(id) {
                return invocation.return_dbus_error(INVALID_ARGS, "unknown menu id");
            }
            invocation.return_value(None);
            click(menu, chosen, id, &event);
        }
        "EventGroup" => {
            let mut errors = Vec::new();
            let events = parameters.child_value(0);
            for event in events.iter() {
                let id = event.child_value(0).get::<i32>().unwrap_or(-1);
                let name = event.child_value(1).str().unwrap_or_default().to_string();
                if menu.contains(id) {
                    click(menu, chosen, id, &name);
                } else {
                    errors.push(id);
                }
            }
            invocation.return_value(Some(&(errors,).to_variant()));
        }
        "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
        "AboutToShowGroup" => {
            let ids = parameters
                .child_value(0)
                .get::<Vec<i32>>()
                .unwrap_or_default();
            let errors: Vec<i32> = ids.into_iter().filter(|id| !menu.contains(*id)).collect();
            invocation.return_value(Some(&(Vec::<i32>::new(), errors).to_variant()));
        }
        _ => invocation
            .return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "no such method"),
    }
}

/// A click on `id`: its item's action, after the call has been answered.
fn click(menu: &MenuLayout, chosen: &Rc<dyn Fn(TrayItem)>, id: i32, event: &str) {
    if !dbusmenu_layout::is_click(event) {
        return;
    }
    let Some(item) = menu.item_for(id) else {
        return;
    };
    crate::fc_info!("tray: menu item {item:?} chosen");
    let chosen = Rc::clone(chosen);
    glib::idle_add_local_once(move || chosen(item));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_introspection_parses_and_names_every_method() {
        let info = dbus::interface(MENU_XML, MENU_INTERFACE);
        for method in [
            "GetLayout",
            "GetGroupProperties",
            "GetProperty",
            "Event",
            "EventGroup",
            "AboutToShow",
            "AboutToShowGroup",
        ] {
            assert!(info.lookup_method(method).is_some(), "{method}");
        }
        assert!(info.lookup_signal("LayoutUpdated").is_some());
        assert_eq!(property("Version").get::<u32>(), Some(3));
        assert_eq!(property("Status").str(), Some("normal"));
        assert_eq!(property("IconThemePath").type_().as_str(), "as");
    }
}
