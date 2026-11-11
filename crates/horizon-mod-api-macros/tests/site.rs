#![forbid(unsafe_code)]
extern crate self as mod_points;
pub enum SiteId {
    One,
    Borrow,
}
pub mod __private {
    pub fn wrap<O>(
        site: super::SiteId,
        at: usize,
        call: impl FnOnce(Option<(*const (), usize)>) -> O,
    ) -> O {
        if matches!(site, super::SiteId::Borrow) && at == 0 {
            call(Some((super::borrow_listener as *const (), 1)))
        } else {
            call(None)
        }
    }
}
struct Game<'a>(&'a mut u32);
struct Args {
    value: u32,
}
type Receiver<'g, 'r> = &'r mut Game<'g>;
type Output = u32;
type Listener = for<'g> fn(
    Receiver<'g, '_>,
    &mut Args,
    &mut dyn FnMut(Receiver<'g, '_>, &mut Args) -> Output,
) -> Output;

#[test]
fn site_macro_reborrows_a_lifetime_parameterized_receiver_without_unsafe_in_the_caller() {
    let mut value = 3;
    let mut game = Game(&mut value);
    let mut args = Args { value: 2 };
    let result = horizon_mod_api_macros::site!(
        __test_site_wrap,
        One,
        Listener,
        Receiver,
        Args,
        Output,
        &mut game,
        args,
        |receiver: Receiver<'_, '_>, args: &mut Args| {
            *receiver.0 += args.value;
            *receiver.0
        }
    );
    assert_eq!(result, 5);
    assert_eq!(value, 5);
}

struct BorrowArgs<'a> {
    folder: &'a str,
    wood: &'a str,
}
type BorrowReceiver<'g, 'r> = ();
type BorrowListener = for<'a> fn(
    (),
    &mut BorrowArgs<'a>,
    &mut dyn FnMut((), &mut BorrowArgs<'a>) -> String,
) -> String;

#[test]
fn borrowed_arguments_live_only_for_the_call() {
    let wood = String::from("oak");
    let mut args = BorrowArgs {
        folder: "signs",
        wood: &wood,
    };
    let value = horizon_mod_api_macros::site!(
        borrowed_wrap,
        Borrow,
        BorrowListener,
        BorrowReceiver,
        BorrowArgs::<'a>,
        String,
        (),
        args,
        |_: (), args: &mut BorrowArgs<'_>| format!("{}/{}", args.folder, args.wood)
    );
    assert_eq!(value, "custom:signs/oak");
}

fn borrow_listener<'a>(
    receiver: (),
    args: &mut BorrowArgs<'a>,
    original: &mut dyn FnMut((), &mut BorrowArgs<'a>) -> String,
) -> String {
    // This is the result-only modify_value adapter; the callback never sees Args.
    fn modify_value(_: (), value: String) -> String {
        format!("custom:{value}")
    }
    assert_eq!(original(receiver, args), "signs/oak");
    modify_value(receiver, original(receiver, args))
}

struct World<'g>(std::cell::Cell<&'g str>);
struct WorldArgs<'g, 'a> {
    world: &'a World<'g>,
    position: [i32; 3],
}
type WorldListener = for<'g, 'a> fn(
    (),
    &mut WorldArgs<'g, 'a>,
    &mut dyn FnMut((), &mut WorldArgs<'g, 'a>) -> bool,
) -> bool;
#[test]
fn engine_type_lifetime_is_not_the_record_borrow_lifetime() {
    let text = String::from("world");
    let world = World(std::cell::Cell::new(text.as_str()));
    let mut args = WorldArgs {
        world: &world,
        position: [1, 2, 3],
    };
    let value = horizon_mod_api_macros::site!(
        world_wrap,
        One,
        WorldListener,
        BorrowReceiver,
        WorldArgs::<'g, 'a>,
        bool,
        (),
        args,
        |_: (), args: &mut WorldArgs<'_, '_>| args.world.0.get() == "world"
            && args.position == [1, 2, 3]
    );
    assert!(value);
}
