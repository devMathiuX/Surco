//! Orden de reproducción de una playlist: siguiente, anterior, shuffle y
//! repetir. No sabe nada de audio: trabaja con índices de canciones.

use surco_core::RepeatMode;

/// Cola sobre una playlist de `len` canciones.
#[derive(Debug, Clone)]
pub struct Queue {
    /// Índices de canciones en el orden en que van a sonar.
    order: Vec<usize>,
    /// Posición actual dentro de `order`.
    pos: usize,
    shuffle: bool,
    repeat: RepeatMode,
    rng: u64,
}

impl Queue {
    /// Cola que arranca en la canción `start`. `seed` alimenta el shuffle.
    pub fn new(len: usize, start: usize, shuffle: bool, repeat: RepeatMode, seed: u64) -> Self {
        let mut q = Self {
            order: (0..len).collect(),
            pos: start.min(len.saturating_sub(1)),
            shuffle: false,
            repeat,
            rng: seed | 1,
        };
        if shuffle {
            q.set_shuffle(true);
        }
        q
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    /// Canción actual.
    pub fn current(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }

    /// Lo que sonaría al terminar la actual (para pre-cargar). Respeta "repetir una".
    pub fn peek_next(&self) -> Option<usize> {
        match self.repeat {
            RepeatMode::One => self.current(),
            _ => self.next_pos(true).map(|p| self.order[p]),
        }
    }

    /// Las próximas `n` canciones en orden (sin repetir la actual), para resolver de antemano.
    pub fn upcoming(&self, n: usize) -> Vec<usize> {
        let mut out = Vec::with_capacity(n);
        let mut p = self.pos;
        for _ in 0..n.min(self.order.len().saturating_sub(1)) {
            p += 1;
            if p >= self.order.len() {
                if self.repeat == RepeatMode::Off {
                    break;
                }
                p = 0;
            }
            out.push(self.order[p]);
        }
        out
    }

    /// Terminó la canción: avanza según el modo de repetición.
    pub fn advance(&mut self) -> Option<usize> {
        if self.repeat == RepeatMode::One {
            return self.current();
        }
        self.step(true)
    }

    /// "Siguiente" o "anterior" a mano: ignora "repetir una".
    pub fn skip(&mut self, forward: bool) -> Option<usize> {
        self.step(forward)
    }

    /// El usuario eligió una canción de la lista.
    pub fn jump(&mut self, index: usize) {
        if let Some(p) = self.order.iter().position(|&i| i == index) {
            self.pos = p;
            if self.shuffle {
                // Que lo que sigue vuelva a ser aleatorio desde acá.
                self.order.swap(0, p);
                self.pos = 0;
                let rest = self.order.len();
                self.shuffle_range(1, rest);
            }
        }
    }

    /// Activa o desactiva el shuffle sin cortar la canción actual.
    pub fn set_shuffle(&mut self, on: bool) {
        let cur = self.current();
        self.shuffle = on;
        if on {
            // La actual queda primera y el resto se mezcla.
            if let Some(c) = cur {
                if let Some(p) = self.order.iter().position(|&i| i == c) {
                    self.order.swap(0, p);
                }
            }
            self.pos = 0;
            let len = self.order.len();
            self.shuffle_range(1, len);
        } else {
            self.order.sort_unstable();
            // Puede faltar alguna (quitada de la cola): buscar la actual.
            self.pos = cur
                .and_then(|c| self.order.iter().position(|&i| i == c))
                .unwrap_or(0);
        }
    }

    /// Orden completo (índices de canciones) y posición de la actual en él.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Agrega la canción `index` al final de la cola.
    pub fn push(&mut self, index: usize) {
        self.order.push(index);
    }

    /// Agrega la canción `index` para que suene después de la actual.
    pub fn insert_next(&mut self, index: usize) {
        let at = (self.pos + 1).min(self.order.len());
        self.order.insert(at, index);
    }

    /// Saca de la cola lo que está en la posición `p` (no la actual).
    pub fn remove_at(&mut self, p: usize) -> Option<usize> {
        if p == self.pos || p >= self.order.len() {
            return None;
        }
        let removed = self.order.remove(p);
        if p < self.pos {
            self.pos -= 1;
        }
        Some(removed)
    }

    /// Mueve lo que está en `from` a la posición `to` (arrastrar en la cola).
    pub fn move_item(&mut self, from: usize, to: usize) {
        let len = self.order.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let cur = self.current();
        let item = self.order.remove(from);
        self.order.insert(to, item);
        if let Some(c) = cur {
            if let Some(p) = self.order.iter().position(|&i| i == c) {
                self.pos = p;
            }
        }
    }

    /// Salta a la posición `p` de la cola (clic en la cola).
    pub fn jump_to_pos(&mut self, p: usize) -> Option<usize> {
        if p >= self.order.len() {
            return None;
        }
        self.pos = p;
        self.current()
    }

    fn next_pos(&self, forward: bool) -> Option<usize> {
        let len = self.order.len();
        if len == 0 {
            return None;
        }
        let wrap = self.repeat != RepeatMode::Off;
        if forward {
            match self.pos + 1 {
                p if p < len => Some(p),
                _ if wrap => Some(0),
                _ => None,
            }
        } else {
            match self.pos.checked_sub(1) {
                Some(p) => Some(p),
                None if wrap => Some(len - 1),
                None => None,
            }
        }
    }

    fn step(&mut self, forward: bool) -> Option<usize> {
        let p = self.next_pos(forward)?;
        // Con shuffle + repetir todas, cada vuelta completa se re-mezcla.
        if forward && p == 0 && self.shuffle && self.order.len() > 2 {
            let last = self.current();
            let len = self.order.len();
            self.shuffle_range(0, len);
            // Que no se repita la misma canción al dar la vuelta.
            if self.order.first().copied() == last {
                self.order.swap(0, len - 1);
            }
        }
        self.pos = p;
        self.current()
    }

    /// Fisher-Yates sobre `order[from..to]` con un xorshift (sin dependencias).
    fn shuffle_range(&mut self, from: usize, to: usize) {
        for i in (from + 1..to).rev() {
            let j = from + (self.rand() % (i - from + 1) as u64) as usize;
            self.order.swap(i, j);
        }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_the_current_song() {
        let mut q = Queue::new(5, 2, false, RepeatMode::Off, 1);
        q.insert_next(9);
        assert_eq!(q.order(), &[0, 1, 2, 9, 3, 4]);
        q.push(7);
        assert_eq!(q.peek_next(), Some(9));
        // No se puede quitar la que suena.
        assert_eq!(q.remove_at(2), None);
        assert_eq!(q.remove_at(0), Some(0));
        assert_eq!(q.current(), Some(2));
        assert_eq!(q.pos(), 1);
        // Mover la actual al final.
        q.move_item(1, 5);
        assert_eq!(q.order(), &[1, 9, 3, 4, 7, 2]);
        assert_eq!(q.current(), Some(2));
        // Mover otra delante de la actual.
        q.move_item(0, 4);
        assert_eq!(q.order(), &[9, 3, 4, 7, 1, 2]);
        assert_eq!(q.current(), Some(2));
        assert_eq!(q.jump_to_pos(0), Some(9));
        assert_eq!(q.upcoming(2), vec![3, 4]);
    }

    #[test]
    fn shuffle_off_after_removing_finds_current() {
        let mut q = Queue::new(6, 4, true, RepeatMode::Off, 3);
        let cur = q.current();
        let other = q.order().iter().position(|&i| Some(i) != cur).unwrap_or(0);
        q.remove_at(other);
        q.set_shuffle(false);
        assert_eq!(q.current(), cur);
        assert_eq!(q.len(), 5);
    }

    fn collect(q: &mut Queue, n: usize) -> Vec<usize> {
        let mut v = vec![q.current().unwrap()];
        for _ in 0..n {
            match q.advance() {
                Some(i) => v.push(i),
                None => break,
            }
        }
        v
    }

    #[test]
    fn linear_without_repeat_stops_at_end() {
        let mut q = Queue::new(4, 1, false, RepeatMode::Off, 1);
        assert_eq!(collect(&mut q, 10), [1, 2, 3]);
        assert_eq!(q.peek_next(), None);
        assert_eq!(q.skip(false), Some(2));
        assert_eq!(q.upcoming(5), [3]);
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_stays() {
        let mut q = Queue::new(3, 1, false, RepeatMode::All, 1);
        assert_eq!(collect(&mut q, 4), [1, 2, 0, 1, 2]);
        assert_eq!(q.upcoming(5), [0, 1]);

        let mut q = Queue::new(3, 1, false, RepeatMode::One, 1);
        assert_eq!(q.peek_next(), Some(1));
        assert_eq!(q.advance(), Some(1));
        // "Siguiente" a mano sí avanza.
        assert_eq!(q.skip(true), Some(2));
        assert_eq!(q.skip(true), Some(0));
    }

    #[test]
    fn previous_at_start() {
        let mut q = Queue::new(3, 0, false, RepeatMode::Off, 1);
        assert_eq!(q.skip(false), None);
        assert_eq!(q.current(), Some(0));
    }

    #[test]
    fn shuffle_plays_everything_once_starting_with_current() {
        let mut q = Queue::new(50, 7, true, RepeatMode::Off, 42);
        let played = collect(&mut q, 100);
        assert_eq!(played[0], 7);
        assert_eq!(played.len(), 50);
        let mut sorted = played.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        assert_ne!(
            played,
            (0..50).collect::<Vec<_>>(),
            "debería estar mezclado"
        );
    }

    #[test]
    fn toggling_shuffle_keeps_current() {
        let mut q = Queue::new(10, 4, false, RepeatMode::Off, 3);
        q.set_shuffle(true);
        assert_eq!(q.current(), Some(4));
        q.advance();
        let cur = q.current().unwrap();
        q.set_shuffle(false);
        assert_eq!(q.current(), Some(cur));
        assert_eq!(q.peek_next(), (cur + 1 < 10).then_some(cur + 1));
    }

    #[test]
    fn jump_and_reshuffle_on_wrap() {
        let mut q = Queue::new(5, 0, true, RepeatMode::All, 9);
        q.jump(3);
        assert_eq!(q.current(), Some(3));
        let first_round = collect(&mut q, 4);
        let next = q.advance().unwrap();
        assert_ne!(Some(next), first_round.last().copied());
    }

    #[test]
    fn empty_queue() {
        let mut q = Queue::new(0, 0, true, RepeatMode::All, 1);
        assert!(q.is_empty());
        assert_eq!(q.current(), None);
        assert_eq!(q.advance(), None);
        assert_eq!(q.peek_next(), None);
    }
}
