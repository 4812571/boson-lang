pub mod alu;
pub mod controls;
pub mod errors;
pub mod ffi;
pub mod frames;
pub mod global;
pub mod stack;
pub mod thread;

use std::cell::RefCell;
use std::rc::Rc;

use controls::Controls;
use errors::VMError;
use errors::VMErrorKind;
use frames::ExecutionFrame;
use global::GlobalPool;
use stack::CallStack;
use stack::DataStack;

use crate::api::Platform;
use crate::compiler::symtab::ConstantPool;
use crate::compiler::CompiledBytecode;
use crate::isa::InstructionKind;
use crate::types::closure;
use crate::types::object;

use object::Object;

// defer - taken from: https://stackoverflow.com/questions/29963449/golang-like-defer-in-rust

struct ScopeCall<F: FnOnce()> {
    c: Option<F>,
}
impl<F: FnOnce()> Drop for ScopeCall<F> {
    fn drop(&mut self) {
        self.c.take().unwrap()()
    }
}

pub struct BosonVM {
    pub constants: ConstantPool,
    pub globals: GlobalPool,
    pub data_stack: DataStack,
    pub call_stack: CallStack,
    pub threads: thread::BosonThreads,
    pub vm_ffi: ffi::BosonFFI,
}

pub struct BosonVMContext<'a> {
    pub platform: &'a mut Platform,
    pub globals: &'a mut GlobalPool,
    pub data_stack: &'a mut DataStack,
    pub call_stack: &'a mut CallStack,
    pub constants: &'a mut ConstantPool,
    pub threads: &'a mut thread::BosonThreads,
    pub vm_ffi: &'a mut ffi::BosonFFI,
}

impl BosonVM {
    pub fn new(bytecode: &CompiledBytecode) -> BosonVM {
        let main_frame = ExecutionFrame::new_from_bytecode(bytecode, "main".to_string(), 0, 0);

        let mut call_stack = CallStack::new();
        let data_stack = DataStack::new();

        let _ = call_stack.push_frame(RefCell::new(main_frame));

        let globals = GlobalPool::new();

        return BosonVM {
            constants: bytecode.constant_pool.clone(),
            call_stack: call_stack,
            data_stack: data_stack,
            globals: globals,
            threads: thread::BosonThreads::new_empty(),
            vm_ffi: ffi::BosonFFI::empty(),
        };
    }

    pub fn new_state(bytecode: &CompiledBytecode, globals: GlobalPool) -> BosonVM {
        let main_frame = ExecutionFrame::new_from_bytecode(bytecode, "main".to_string(), 0, 0);

        let mut call_stack = CallStack::new();
        let data_stack = DataStack::new();

        let _ = call_stack.push_frame(RefCell::new(main_frame));

        return BosonVM {
            constants: bytecode.constant_pool.clone(),
            call_stack: call_stack,
            data_stack: data_stack,
            globals: globals,
            threads: thread::BosonThreads::new_empty(),
            vm_ffi: ffi::BosonFFI::empty(),
        };
    }

    pub fn new_empty_from_state(globals: GlobalPool, constants: ConstantPool) -> BosonVM {
        let call_stack = CallStack::new();
        let data_stack = DataStack::new();

        return BosonVM {
            constants: constants,
            call_stack: call_stack,
            data_stack: data_stack,
            globals: globals,
            threads: thread::BosonThreads::new_empty(),
            vm_ffi: ffi::BosonFFI::empty(),
        };
    }

    pub fn push_new_frame(&mut self, frame: RefCell<ExecutionFrame>) -> Option<VMError> {
        let push_result = self.call_stack.push_frame(frame);
        if push_result.is_err() {
            return Some(push_result.unwrap_err());
        }

        return None;
    }

    pub fn eval_bytecode(
        &mut self,
        platform: &mut Platform,
        pop_last: bool,
        break_on_ret: bool,
    ) -> Result<Rc<Object>, VMError> {
        let mut context = BosonVMContext {
            platform: platform,
            globals: &mut self.globals,
            constants: &mut self.constants,
            data_stack: &mut self.data_stack,
            call_stack: &mut self.call_stack,
            threads: &mut self.threads,
            vm_ffi: &mut self.vm_ffi,
        };

        return Self::eval_bytecode_from_context(&mut context, pop_last, break_on_ret);
    }

    pub fn dump_globals(&self) -> String {
        let mut result = String::new();
        let mut idx = 0;
        for obj in &self.globals.pool {
            match obj.as_ref() {
                Object::Noval => {}
                _ => {
                    let repr = obj.as_ref().describe();
                    result.push_str(&format!("{:0>8x} {}\n", idx, repr));
                    idx += 1;
                }
            }
        }

        return result;
    }

    pub fn execute_sandbox(
        closure: Rc<closure::ClosureContext>,
        params: Vec<Rc<Object>>,
        platform: &mut Platform,
        globals: GlobalPool,
        constants: ConstantPool,
    ) -> Result<Rc<Object>, VMError> {
        // create an execution frame for that closure:

        // new empty from state will create a VM with an empty call stack.
        let mut vm_instance = BosonVM::new_empty_from_state(globals, constants);
        let closure_rc = Rc::new(Object::ClosureContext(closure));

        // push the arguments on top of the stack:
        let params_len = params.len();
        let mut error = Controls::push_objects(params, &mut vm_instance.data_stack);
        if error.is_some() {
            return Err(error.unwrap());
        }

        // push the closure on to the stack:
        error = Controls::push_objects(vec![closure_rc], &mut vm_instance.data_stack);
        if error.is_some() {
            return Err(error.unwrap());
        }

        // execute the function call:
        let exec_frame = Controls::execute_call(
            &InstructionKind::ICall,
            &mut vm_instance.data_stack,
            params_len,
            &mut vm_instance.globals,
            &mut vm_instance.constants,
            platform,
            &mut vm_instance.threads,
            &mut vm_instance.vm_ffi,
        );

        if exec_frame.is_err() {
            return Err(exec_frame.unwrap_err());
        }

        // check if there is a frame returned:
        let unwrapped_frame_res = exec_frame.unwrap();
        if unwrapped_frame_res.is_some() {
            // custom function call, push the execution frame:
            let frame_push_res = vm_instance.push_new_frame(unwrapped_frame_res.unwrap());
            if frame_push_res.is_some() {
                return Err(frame_push_res.unwrap());
            }
            // evaluate the frame:
            let eval_result = vm_instance.eval_bytecode(platform, true, true);
            return eval_result;
        }

        // last item in the stack:
        let popped_result = vm_instance
            .data_stack
            .pop_object(InstructionKind::IBlockEnd);
        if popped_result.is_err() {
            return Ok(Rc::new(Object::Noval));
        }

        return Ok(popped_result.unwrap());
    }

    pub fn dump_ds(&self) -> String {
        let mut result = String::new();
        let mut idx = 0;
        for obj in &self.data_stack.stack {
            let repr = obj.as_ref().describe();
            result.push_str(&format!("{:0>8x} {}\n", idx, repr));
            idx += 1;
        }

        return result;
    }

    pub fn eval_bytecode_from_context(
        context: &mut BosonVMContext,
        pop_last: bool,
        break_on_ret: bool,
    ) -> Result<Rc<Object>, VMError> {
        while context.call_stack.top_ref().has_instructions() {
            let mut frame = context.call_stack.top();

            let (inst, operands, next) = frame.read_current_instruction();

            match inst {
                // illegal and NoOp
                InstructionKind::INoOp => {
                    frame.farword_ip(next);
                }

                InstructionKind::IIllegal => {
                    return Err(VMError::new(
                        "VM encountered illegal instruction".to_string(),
                        VMErrorKind::IllegalOperation,
                        Some(InstructionKind::IIllegal),
                        0,
                    ));
                }

                InstructionKind::IBlockStart | InstructionKind::IBlockEnd => {
                    frame.farword_ip(next);
                }

                // jump and not jump
                InstructionKind::IJump => {
                    let pos = operands[0];
                    let result = Controls::jump(&mut frame, pos);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }
                }

                InstructionKind::INotJump => {
                    let pos = operands[0];
                    let result =
                        Controls::jump_not_truthy(&mut frame, &mut context.data_stack, pos);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    let has_jumped = result.unwrap();
                    if !has_jumped {
                        frame.farword_ip(next);
                    }
                }

                // data load and store instructions:
                InstructionKind::IConstant => {
                    let const_pos = operands[0];
                    let result = Controls::load_constant(
                        &context.constants,
                        &mut context.data_stack,
                        const_pos,
                    );

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IStoreGlobal => {
                    let store_pos = operands[0];
                    let result = Controls::store_global(
                        &mut context.globals,
                        &mut context.data_stack,
                        store_pos,
                    );

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadGlobal => {
                    let store_pos = operands[0];
                    let result = Controls::load_global(
                        &mut context.globals,
                        &mut context.data_stack,
                        store_pos,
                    );

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadFree => {
                    let store_pos = operands[0];
                    let error = Controls::load_free(&mut context.data_stack, &mut frame, store_pos);

                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadLocal => {
                    let store_pos = operands[0];
                    let result =
                        Controls::load_local(&mut context.data_stack, store_pos, &mut frame);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IStoreLocal => {
                    let store_pos = operands[0];
                    let result =
                        Controls::store_local(&mut context.data_stack, store_pos, &mut frame);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IAssertFail => {
                    let error = Controls::raise_assertion_error(&mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IGetIndex => {
                    let error = Controls::get_index_value(&mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ISetIndex => {
                    let error = Controls::set_indexed(&mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // Binary operations:
                InstructionKind::IAdd
                | InstructionKind::ISub
                | InstructionKind::IMul
                | InstructionKind::IDiv
                | InstructionKind::IMod
                | InstructionKind::IAnd
                | InstructionKind::IOr
                | InstructionKind::ILAnd
                | InstructionKind::ILOr
                | InstructionKind::ILGt
                | InstructionKind::ILGte
                | InstructionKind::ILLTe
                | InstructionKind::ILLt
                | InstructionKind::ILEq
                | InstructionKind::ILNe => {
                    let error = Controls::execute_binary_op(&inst, &mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // unary operators:
                InstructionKind::ILNot | InstructionKind::INeg => {
                    let error = Controls::execute_unary_op(&inst, &mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // built-ins
                InstructionKind::ILoadBuiltIn => {
                    let builtin_idx = operands[0];
                    let result = Controls::load_builtin(&mut context.data_stack, builtin_idx);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                // function call:
                InstructionKind::ICall => {
                    let args_len = operands[0];

                    let result = Controls::execute_call(
                        &inst,
                        &mut context.data_stack,
                        args_len,
                        &mut context.globals,
                        &mut context.constants,
                        context.platform,
                        &mut context.threads,
                        &mut context.vm_ffi,
                    );

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    let new_frame = result.unwrap();
                    if new_frame.is_some() {
                        // the previous frame should point to the
                        // next instruction after call
                        frame.farword_ip(next);
                        // Looking for better way to handle this:
                        std::mem::drop(frame);
                        // -------------------------------------
                        let push_result = context.call_stack.push_frame(new_frame.unwrap());
                        if push_result.is_err() {
                            return Err(push_result.unwrap_err());
                        }
                    } else {
                        frame.farword_ip(next);
                    }
                }

                InstructionKind::ICallThread => {
                    let n_args = operands[0];
                    let result = Controls::execute_thread(
                        &inst,
                        &mut context.data_stack,
                        n_args,
                        &mut context.globals,
                        &mut context.constants,
                        context.platform,
                        &mut context.threads,
                        false,
                    );

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ICallAsync => {
                    let n_args = operands[0];
                    let result = Controls::execute_thread(
                        &inst,
                        &mut context.data_stack,
                        n_args,
                        &mut context.globals,
                        &mut context.constants,
                        context.platform,
                        &mut context.threads,
                        true,
                    );

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // build Array and Hash:
                InstructionKind::IArray => {
                    let length = operands[0];
                    let result = Controls::build_array(&inst, &mut context.data_stack, length);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IHash => {
                    let length = operands[0];
                    let result = Controls::build_hash(&inst, &mut context.data_stack, length);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IClosure => {
                    let error = Controls::create_closure(
                        &mut context.data_stack,
                        &context.constants,
                        operands[1],
                        operands[0],
                    );

                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IRet => {
                    std::mem::drop(frame);
                    let current_frame_res = context.call_stack.pop_frame();
                    if current_frame_res.is_err() {
                        return Err(current_frame_res.unwrap_err());
                    }

                    // execute return: This function cleans up the subroutine's data
                    // on stack
                    let error = Controls::execute_return(
                        &mut context.data_stack,
                        &current_frame_res.unwrap().borrow(),
                        false,
                    );

                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    if break_on_ret {
                        break;
                    }
                }

                InstructionKind::IRetVal => {
                    std::mem::drop(frame);
                    let current_frame_res = context.call_stack.pop_frame();
                    if current_frame_res.is_err() {
                        return Err(current_frame_res.unwrap_err());
                    }

                    // execute return: This function cleans up the subroutine's data
                    // on stack
                    let error = Controls::execute_return(
                        &mut context.data_stack,
                        &current_frame_res.unwrap().borrow(),
                        true,
                    );

                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    if break_on_ret {
                        break;
                    }
                }

                InstructionKind::IIter => {
                    let error = Controls::create_iter(&mut context.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IIterNext => {
                    let jmp_pos = operands[0];
                    let result = Controls::jump_next_iter(
                        &mut context.data_stack,
                        jmp_pos,
                        &mut frame,
                        false,
                    );
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    let has_jumped = result.unwrap();
                    if !has_jumped {
                        frame.farword_ip(next);
                    }
                }

                InstructionKind::IEnumNext => {
                    let jmp_pos = operands[0];
                    let result = Controls::jump_next_iter(
                        &mut context.data_stack,
                        jmp_pos,
                        &mut frame,
                        true,
                    );
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    let has_jumped = result.unwrap();
                    if !has_jumped {
                        frame.farword_ip(next);
                    }
                }

                InstructionKind::IShell => {
                    let result = Controls::exec_shell(
                        &inst,
                        &mut context.data_stack,
                        context.platform,
                        &mut context.globals,
                        &mut context.constants,
                        &mut context.threads,
                        &mut context.vm_ffi,
                        false,
                    );

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IShellRaw => {
                    let result = Controls::exec_shell(
                        &inst,
                        &mut context.data_stack,
                        context.platform,
                        &mut context.globals,
                        &mut context.constants,
                        &mut context.threads,
                        &mut context.vm_ffi,
                        true,
                    );

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IGetAttr => {
                    let n_attrs = operands[0];
                    let result = Controls::get_attr(&mut context.data_stack, &inst, n_attrs);

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ICallAttr => {
                    let n_attrs = operands[0];
                    let n_params = operands[1];

                    let result = Controls::call_attr(
                        &mut context.data_stack,
                        &inst,
                        n_attrs,
                        n_params,
                        context.platform,
                        &mut context.globals,
                        &mut context.constants,
                        &mut context.threads,
                        &mut context.vm_ffi,
                    );

                    if result.is_some() {
                        return Err(result.unwrap());
                    }

                    frame.farword_ip(next);
                }

                _ => {
                    return Err(VMError::new(
                        format!("{} not yet implemented", inst.as_string()),
                        VMErrorKind::InstructionNotImplemented,
                        Some(inst),
                        0,
                    ));
                }
            }
        }

        if pop_last {
            let popped_result = context.data_stack.pop_object(InstructionKind::IBlockEnd);
            if popped_result.is_err() {
                return Ok(Rc::new(Object::Noval));
            }
            return Ok(popped_result.unwrap());
        }

        return Ok(Rc::new(Object::Noval));
    }
}
