use std::sync::atomic::AtomicUsize;

use crate::util::abort;

struct WeakRc {
    strong: AtomicUsize,
    weak: AtomicUsize,
}

const NO_WEAK: usize = 1;
const WEAK_ONE: usize = 2;
const DESTRUCTED: usize = 1 << (usize::BITS - 1);
const LIMIT: usize = DESTRUCTED >> 1;

impl WeakRc {


    fn new() -> Self {
       Self {
            strong: AtomicUsize::new(1),
            weak: AtomicUsize::new(NO_WEAK + WEAK_ONE),
        }
    }

    fn incr_strong(&self) {
        let val = self.strong.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if val >= LIMIT {
            // FIXME: if DESTRUCTED is set just NOP (but we only have to do this if we incr_strong on upgrade)
            abort();
        }
    }

    fn decr_strong(&self) {
        let val = self.strong.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        if val == 1 {
            self.drop_slow();
        }
    }

    fn drop_slow(&self) {
        let weak = self.weak.load(std::sync::atomic::Ordering::Acquire);
        if weak == (NO_WEAK + WEAK_ONE) {
            // drop + dealloc!
            return;
        }
        if let Ok(_) = self.strong.compare_exchange(0, DESTRUCTED, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Relaxed) {
            // destruct
            if self.weak.fetch_sub(WEAK_ONE, std::sync::atomic::Ordering::Acquire) == WEAK_ONE {
                // dealloc
            }
        }     
    }

    fn incr_weak(&self) {
        let val = self.weak.fetch_add(WEAK_ONE, std::sync::atomic::Ordering::AcqRel);
        if val >= LIMIT {
            abort();
        }
        if val == (NO_WEAK + WEAK_ONE) {
            // unset NO_WEAK flag
            self.weak.fetch_sub(NO_WEAK, std::sync::atomic::Ordering::AcqRel);
        }
    }

    fn decr_weak(&self) {
        let val = self.weak.fetch_sub(WEAK_ONE, std::sync::atomic::Ordering::AcqRel);
        if val == WEAK_ONE {
            // free!
        }
    }

    fn upgrade(&self) {
        let strong = self.strong.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if strong & DESTRUCTED != 0 {
            // we detected a failure
            return;
        }
        todo!()
    }

    fn downgrade(&self) {
        self.incr_weak();
        self.decr_strong();
    }

    fn get_mut(&mut self) {

    }

    fn into_inner(self) {
        
    }


}