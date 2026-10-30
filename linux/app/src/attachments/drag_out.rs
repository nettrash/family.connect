//! Dragging an attachment OUT of a conversation to the file manager (the Mac's `DraggedAttachment`)
//! — the other half of drag and drop. Nothing new goes over the wire: it is the original's
//! `GET /attachments/{id}`, the request Save… already makes.
//!
//! A PROMISE, NOT A PATH. What a bubble has drawn is mostly the PREVIEW, and an original may not be
//! on this device at all; a path handed over when the drag starts would drop a thumbnail wearing
//! the photograph's name. So the drag carries a [`Promise`]: the receiver asks for its
//! `text/uri-list` when the drop lands, and only then is the original fetched, written under its
//! real name in a folder of its own among the temporaries (swept at launch), and its address given.
//! The receiver gets the original or it gets nothing — a fetch that fails is an error, never a
//! preview and never an empty file.

use std::future::Future;
use std::pin::Pin;

use adw::prelude::*;
use adw::subclass::prelude::*;
use fc_core::protocol::AttachmentDto;
use gtk::gdk;

use crate::shell::Shell;

/// Lets `widget` be dragged out as `attachment`'s file.
pub fn attach(widget: &impl IsA<gtk::Widget>, attachment: &AttachmentDto) {
    let source = gtk::DragSource::builder()
        .actions(gdk::DragAction::COPY)
        .build();
    let held = attachment.clone();
    source.connect_prepare(move |_, _, _| Some(Promise::new(&held).upcast()));
    let face = widget.as_ref().downgrade();
    source.connect_drag_begin(move |source, _| {
        if let Some(face) = face.upgrade() {
            source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&face))), 0, 0);
        }
    });
    widget.as_ref().add_controller(source);
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    pub struct Promise {
        pub attachment: RefCell<Option<AttachmentDto>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Promise {
        const NAME: &'static str = "FcAttachmentPromise";
        type Type = super::Promise;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for Promise {}

    impl gdk::subclass::prelude::ContentProviderImpl for Promise {
        fn formats(&self) -> gdk::ContentFormats {
            gdk::ContentFormats::new(&[URI_LIST])
        }

        fn write_mime_type_future(
            &self,
            mime_type: &str,
            stream: &gio::OutputStream,
            io_priority: glib::Priority,
        ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
            let attachment = self.attachment.borrow().clone();
            let (wanted, stream) = (mime_type == URI_LIST, stream.clone());
            Box::pin(async move {
                let attachment = attachment.filter(|_| wanted).ok_or_else(|| {
                    glib::Error::new(gio::IOErrorEnum::NotSupported, "not a file list")
                })?;
                let path = super::fetched(&attachment).await?;
                let line = format!("{}\r\n", gio::File::for_path(&path).uri());
                stream
                    .write_all_future(line.into_bytes(), io_priority)
                    .await
                    .map_err(|(_, error)| error)?;
                Ok(())
            })
        }
    }
}

glib::wrapper! {
    /// The drag's content: one attachment's original, fetched when the drop asks for it.
    pub struct Promise(ObjectSubclass<imp::Promise>) @extends gdk::ContentProvider;
}

impl Promise {
    fn new(attachment: &AttachmentDto) -> Promise {
        let promise: Promise = glib::Object::new();
        *promise.imp().attachment.borrow_mut() = Some(attachment.clone());
        promise
    }
}

/// What a file manager asks a drop for.
const URI_LIST: &str = "text/uri-list";

/// The original, on this device under its real name — or why it is not.
async fn fetched(attachment: &AttachmentDto) -> Result<std::path::PathBuf, glib::Error> {
    let failed = |what: &str| glib::Error::new(gio::IOErrorEnum::Failed, what);
    let shell = Shell::get();
    let connection = shell.connection().ok_or_else(|| failed("signed out"))?;
    let bytes = connection
        .attachments
        .bytes(attachment, false)
        .await
        .map_err(|error| {
            crate::fc_info!("drag out: the original was refused ({})", error.code);
            failed("the original could not be fetched")
        })?;
    if bytes.is_empty() {
        return Err(failed("nothing to write"));
    }
    let folder =
        shell
            .paths
            .tmp_dir()
            .join(format!("drag-{}-{}", attachment.id, std::process::id()));
    let path = folder.join(super::saving::suggested_name(attachment));
    std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::write(&path, &bytes))
        .map_err(|error| failed(&error.kind().to_string()))?;
    crate::fc_info!("drag out: attachment {} handed over", attachment.id);
    Ok(path)
}
