#![feature(associated_consts)]
#![feature(box_patterns)]
#![feature(box_syntax)]
#![feature(plugin)]

#![plugin(clippy)]

extern crate uuid;
extern crate jsrs_common;
extern crate js_types;

#[macro_use] extern crate matches;

pub mod alloc;
mod gc_error;
mod scope;
mod test_utils;

use std::cell::RefCell;
use std::mem;
use std::rc::Rc;

use jsrs_common::ast::Exp;
use js_types::js_var::{JsPtrEnum, JsVar};
use js_types::binding::Binding;

use alloc::AllocBox;
use gc_error::{GcError, Result};
use scope::{Scope, ScopeTag};

pub struct ScopeManager {
    globals: Scope,
    curr_scope: Scope,
    closures: Vec<Scope>,
    alloc_box: Rc<RefCell<AllocBox>>,
}

impl ScopeManager {
    fn new(alloc_box: Rc<RefCell<AllocBox>>) -> ScopeManager {
        ScopeManager {
            globals: Scope::new(ScopeTag::Call, &alloc_box),
            curr_scope: Scope::new(ScopeTag::Call, &alloc_box),
            closures: Vec::new(),
            alloc_box: alloc_box,
        }
    }

    pub fn push_scope(&mut self, exp: &Exp) {
        let tag = match *exp {
            Exp::Call(..) => ScopeTag::Call,
            _ => ScopeTag::Block,
        };
        let parent = mem::replace(&mut self.curr_scope, Scope::new(tag, &self.alloc_box));
        self.curr_scope.set_parent(parent);
    }

    pub fn pop_scope(&mut self, gc_yield: bool) -> Result<()> {
        let parent = try!(self.curr_scope.transfer_stack(&mut self.closures, gc_yield));
        if let Some(parent) = parent {
            mem::replace(&mut self.curr_scope, *parent);
            Ok(())
        } else {
            Err(GcError::Scope)
        }
    }

    pub fn alloc(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> Result<()> {
        self.curr_scope.push_var(var, ptr)
    }

    /// Try to load the variable behind a binding
    pub fn load(&self, bnd: &Binding) -> Result<(JsVar, Option<JsPtrEnum>)> {
        self.curr_scope.get_var_copy(bnd)
                       // Binding lookup failed locally, so check the root
                       // scope (globals)
                       .or(self.globals.get_var_copy(bnd))
                       .ok_or_else(|| GcError::Load(bnd.clone()))
    }

    pub fn store(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> Result<()> {
        let update = self.curr_scope.update_var(var, ptr);
        if let Err(GcError::Store(var, ptr)) = update {
            self.alloc(var, ptr)
        } else {
            update
        }
    }
}

pub fn init_gc() -> ScopeManager {
    let alloc_box = Rc::new(RefCell::new(AllocBox::new()));
    ScopeManager::new(alloc_box)
}


#[cfg(test)]
mod tests {
    use super::*;

    use jsrs_common::ast::Exp;
    use js_types::js_var::JsType;
    use js_types::binding::Binding;

    use gc_error::GcError;
    use test_utils;

    #[test]
    fn test_pop_scope() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        mgr.push_scope(&Exp::Undefined);
        assert!(mgr.curr_scope.parent.is_some());
        mgr.pop_scope(false).unwrap();
        assert!(mgr.curr_scope.parent.is_none());
    }

    #[test]
    fn test_pop_scope_fail() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let res = mgr.pop_scope(false);
        assert!(res.is_err());
        assert!(matches!(res, Err(GcError::Scope)));
    }

    #[test]
    fn test_alloc() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        mgr.alloc(test_utils::make_num(1.), None).unwrap();
        mgr.push_scope(&Exp::Undefined);
        mgr.alloc(test_utils::make_num(2.), None).unwrap();
        assert!(mgr.alloc_box.borrow().is_empty());
    }

    #[test]
    fn test_load() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let x = test_utils::make_num(1.);
        let x_bnd = x.binding.clone();
        mgr.alloc(x, None).unwrap();
        let load = mgr.load(&x_bnd);
        assert!(load.is_ok());
        let load = load.unwrap();
        match load.0.t {
            JsType::JsNum(n) => assert!(f64::abs(n - 1.) < 0.0001),
            _ => unreachable!(),
        }
        assert!(load.1.is_none());
    }

    #[test]
    fn test_load_fail() {
        let alloc_box = test_utils::make_alloc_box();
        let mgr = ScopeManager::new(alloc_box);
        let bnd = Binding::anon();
        let res = mgr.load(&bnd);
        assert!(res.is_err());
        assert!(matches!(res, Err(GcError::Load(_))));
        if let Err(GcError::Load(res_bnd)) = res {
            assert_eq!(bnd, res_bnd);
        }
    }

    #[test]
    fn test_store() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box,);
        mgr.push_scope(&Exp::Undefined);
        let x = test_utils::make_num(1.);
        let x_bnd = x.binding.clone();
        mgr.alloc(x, None).unwrap();

        let (mut var, _) = mgr.load(&x_bnd).unwrap();
        var.t = JsType::JsNum(2.);

        assert!(mgr.store(var, None).is_ok());
    }

    #[test]
    fn test_store_failed_store() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box,);
        let x = test_utils::make_num(1.);
        let x_bnd = x.binding.clone();
        assert!(mgr.store(x, None).is_ok());

        let load = mgr.load(&x_bnd);
        assert!(load.is_ok());
        let (var, ptr) = load.unwrap();

        assert!(matches!(var.t, JsType::JsNum(1.)));
        assert!(ptr.is_none());
    }

}
