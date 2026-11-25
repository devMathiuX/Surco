//! Ícono en la bandeja del sistema (solo Windows): cerrar la ventana con la
//! X la esconde y Surco sigue sonando ahí. Desde el ícono se vuelve a abrir
//! o se sale del todo.

use iced::futures::Stream;

use crate::Message;

/// Lo que se pide desde el ícono de la bandeja.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMsg {
    Show,
    TogglePause,
    Quit,
}

#[cfg(windows)]
mod imp {
    use std::sync::{Mutex, OnceLock};

    use tokio::sync::mpsc;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::TrayMsg;
    use crate::tr;

    pub static EVENTS: OnceLock<Mutex<Option<mpsc::UnboundedReceiver<TrayMsg>>>> = OnceLock::new();

    /// Mantiene vivo el ícono (al soltarlo desaparece).
    pub struct Tray(#[allow(dead_code)] TrayIcon);

    impl std::fmt::Debug for Tray {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Tray")
        }
    }

    pub fn create() -> Option<Tray> {
        let img = ::image::load_from_memory_with_format(crate::ICON_PNG, ::image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let icon = tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()?;

        let open = MenuItem::with_id("open", tr!("Abrir Surco", "Open Surco"), true, None);
        let toggle = MenuItem::with_id(
            "toggle",
            tr!("Reproducir / pausar", "Play / pause"),
            true,
            None,
        );
        let quit = MenuItem::with_id("quit", tr!("Salir", "Quit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &toggle, &PredefinedMenuItem::separator(), &quit])
            .ok()?;

        let tray = match TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Surco")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo crear el ícono de la bandeja");
                return None;
            }
        };

        let (tx, rx) = mpsc::unbounded_channel();
        let _ = EVENTS.set(Mutex::new(Some(rx)));
        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let msg = match e.id.as_ref() {
                "open" => TrayMsg::Show,
                "toggle" => TrayMsg::TogglePause,
                "quit" => TrayMsg::Quit,
                _ => return,
            };
            let _ = menu_tx.send(msg);
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let show = matches!(
                e,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick { .. }
            );
            if show {
                let _ = tx.send(TrayMsg::Show);
            }
        }));
        Some(Tray(tray))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::{Mutex, OnceLock};

    use super::TrayMsg;

    pub static EVENTS: OnceLock<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<TrayMsg>>>> =
        OnceLock::new();

    #[derive(Debug)]
    pub struct Tray;

    /// En Linux y macOS la X cierra Surco como siempre.
    pub fn create() -> Option<Tray> {
        None
    }
}

pub use imp::{create, Tray};

/// Eventos del ícono de la bandeja, como mensajes.
pub fn events() -> impl Stream<Item = Message> {
    let rx = imp::EVENTS.get().and_then(|m| m.lock().ok()?.take());
    iced::futures::stream::unfold(rx, |rx| async move {
        let mut rx = rx?;
        let msg = rx.recv().await?;
        Some((Message::Tray(msg), Some(rx)))
    })
}
