//! `host.transaction` (#41 §7): NeoForge 26.3's `Transaction`. Changes made through the
//! capability helpers inside one are kept by the game and made final only when the outermost
//! transaction commits; an abort puts back everything its closure changed.
use crate::ServerHost;

/// What the game's raw transaction helpers take, and only `host.transaction`
/// makes: a mod opens and closes transactions through it alone, so its drop guard closes every
/// one a mod opens.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct TransactionSeal(pub(crate) ());

/// An open transaction, as NeoForge's `TransactionContext`: what every write of a handler or a
/// place takes, so none happens outside one. Only `host.transaction` makes one, and lends it to
/// its closure alone.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct Transaction(pub(crate) ());

/// Transactions are available only on the logical server and preserve the receiver type.
///
/// ```
/// use mod_api::{BlockEntityError, BlockEntityRef, HostTransaction, ServerHost};
/// fn write(host: &mut dyn ServerHost, block: BlockEntityRef) {
///     let _: Result<(), BlockEntityError> = host.transaction(|host, _tx| {
///         block.set_changed(host)?;
///         Ok(())
///     });
/// }
/// ```
/// ```compile_fail
/// use mod_api::{BlockEntityError, BlockEntityRef, Host, HostTransaction};
/// fn write(host: &mut dyn Host, block: BlockEntityRef) {
///     let _: Result<(), BlockEntityError> = host.transaction(|host, _tx| {
///         block.set_changed(host)?;
///         Ok(())
///     });
/// }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, HostTransaction};
/// fn client(host: &mut dyn ClientHost) {
///     let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait HostTransaction: ServerHost {
    /// Runs `f` in a transaction, nested in the open one if any (`Transaction.openRoot` and
    /// `open`): `Ok` commits it (`commit`), into the transaction around it or, at the
    /// outermost, for good, with the world told of each change once; `Err` aborts it
    /// (`close` without `commit`), putting every storage it wrote back as it was. A panic
    /// unwinding out of `f` aborts this transaction as it passes, so whoever catches it sees
    /// nothing `f` changed, and the transaction around it stays open.
    ///
    /// Covered: block entities (item slots, saved compound, custom name and attachments), cauldrons,
    /// players' inventories and cursors, entities' inventories
    /// ([`ItemPlace::EntitySlot`](crate::ItemPlace::EntitySlot)) and item entities' stacks.
    fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut Self, &Transaction) -> Result<T, E>,
    ) -> Result<T, E>;
}

impl<H: ServerHost + ?Sized> HostTransaction for H {
    fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut Self, &Transaction) -> Result<T, E>,
    ) -> Result<T, E> {
        run_transaction(self, f)
    }
}

/// `host.transaction` for a host of a known type, which `f` gets back: the game's own moves (a
/// hopper's) run in one through this.
pub fn run_transaction<H: ServerHost + ?Sized, T, E>(
    host: &mut H,
    f: impl FnOnce(&mut H, &Transaction) -> Result<T, E>,
) -> Result<T, E> {
    /// Closes its transaction when dropped: committed when `f` gave `Ok`, else aborted, as on
    /// a panic unwinding past it.
    struct Open<'a, H: ServerHost + ?Sized> {
        host: &'a mut H,
        commit: bool,
    }
    impl<H: ServerHost + ?Sized> Drop for Open<'_, H> {
        fn drop(&mut self) {
            self.host
                .transaction_close(TransactionSeal(()), self.commit);
        }
    }
    host.transaction_open(TransactionSeal(()));
    let mut open = Open {
        host,
        commit: false,
    };
    let result = f(&mut *open.host, &Transaction(()));
    open.commit = result.is_ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHost;

    #[test]
    fn a_transaction_commits_on_ok_and_aborts_on_err_and_nests() {
        let mut fake = FakeHost::new();
        let host: &mut dyn ServerHost = &mut fake;
        let outer = host.transaction(|host, _| {
            let inner: Result<(), &str> = host.transaction(|_, _| Err("refused"));
            assert_eq!(inner, Err("refused"));
            host.transaction(|_, _| Ok::<_, ()>(7))
        });
        assert_eq!(outer, Ok(7));
        assert_eq!(fake.journal.closed(), [(2, false), (2, true), (1, true)]);
        assert_eq!(fake.journal.depth(), 0);
    }

    #[test]
    fn a_panic_aborts_exactly_the_transaction_it_leaves() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut fake = FakeHost::new();
        let host: &mut dyn ServerHost = &mut fake;
        // A child's panic caught inside the root: the child aborts, the root still commits.
        let root = host.transaction(|host, _| {
            let child = catch_unwind(AssertUnwindSafe(|| {
                host.transaction(|_, _| -> Result<(), ()> { panic!("a broken handler") })
            }));
            assert!(child.is_err());
            Ok::<_, ()>(())
        });
        assert_eq!(root, Ok(()));
        // A root's panic caught outside it: the root aborts before the catch sees anything.
        let host: &mut dyn ServerHost = &mut fake;
        let caught = catch_unwind(AssertUnwindSafe(|| {
            host.transaction(|_, _| -> Result<(), ()> { panic!("a broken handler") })
        }));
        assert!(caught.is_err());
        assert_eq!(fake.journal.closed(), [(2, false), (1, true), (1, false)]);
        assert_eq!(fake.journal.depth(), 0);
    }
}
