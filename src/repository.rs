use std::{io, ops::Deref};

/// Something that can be stored in a respository
pub trait Entity: Sized {
    /// Events produced by write operatoions on the repository
    type WriteEvent;

    /// Add events to the entity to make a [`WriteResult`]
    fn with_events<const N: usize>(self, events: [Option<Self::WriteEvent>; N]) -> WriteResult<Self, N> {
        Ok(WithEvents {
            entity: self,
            events: Events(events),
        })
    }
}

/// An `Entity` written to the repository with up to `N` events caused by the write operation
pub struct WithEvents<T: Entity, const N: usize> {
    entity: T,
    events: Events<T::WriteEvent, N>,
}

impl<T: Entity, const N: usize> WithEvents<T, N> {
    pub fn into_events(self) -> Events<T::WriteEvent, N> {
        self.events
    }
}

impl<T: Entity, const N: usize> Deref for WithEvents<T, N> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.entity
    }
}

/// An iterator over a limited number of events, as the number of events is limited, this iterator trades a higher cost of iteration to avoid allocation
pub struct Events<Ev, const N: usize>(pub [Option<Ev>; N]);

impl<Ev, const N: usize> Iterator for Events<Ev, N> {
    type Item = Ev;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.iter_mut().find(|e| e.is_some()).and_then(Option::take)
    }
}

/// The result of a write operation on a database, on success contains up to `N` entity events representing what changed
pub type WriteResult<T, const N: usize> = io::Result<WithEvents<T, N>>;
