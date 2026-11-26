//! Saludo del inicio según la hora del día.

use std::sync::OnceLock;

use time::{OffsetDateTime, UtcOffset};

/// Diferencia con UTC, leída al arrancar (`None`: no se pudo saber).
static OFFSET: OnceLock<Option<UtcOffset>> = OnceLock::new();

/// Lee la zona horaria. Va al principio de `main`, con un solo hilo.
pub fn init() {
    let _ = OFFSET.set(UtcOffset::current_local_offset().ok());
}

/// Hora local (0-23), si se conoce la zona horaria.
fn local_hour() -> Option<u8> {
    let offset = (*OFFSET.get()?)?;
    Some(OffsetDateTime::now_utc().to_offset(offset).hour())
}

/// Saludo para la hora `hour`.
fn for_hour(hour: u8) -> &'static str {
    match hour {
        5..=11 => tr!("¡Buen día!", "Good morning!"),
        12..=19 => tr!("¡Buenas tardes!", "Good afternoon!"),
        _ => tr!("¡Buenas noches!", "Good evening!"),
    }
}

/// Saludo de ahora.
pub fn now() -> &'static str {
    match local_hour() {
        Some(h) => for_hour(h),
        None => tr!("¡Hola de nuevo!", "Welcome back!"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greets_by_time_of_day() {
        crate::i18n::set(crate::i18n::Lang::Es);
        assert_eq!(for_hour(7), "¡Buen día!");
        assert_eq!(for_hour(15), "¡Buenas tardes!");
        assert_eq!(for_hour(22), "¡Buenas noches!");
        assert_eq!(for_hour(3), "¡Buenas noches!");
    }
}
