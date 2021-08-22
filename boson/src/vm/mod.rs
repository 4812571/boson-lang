pub mod alu;
pub mod controls;
pub mod errors;
pub mod frames;
pub mod global;
pub mod stack;

use std::cell::RefCell;
use std::rc::Rc;

use controls::Controls;
use errors::VMError;
use errors::VMErrorKind;
use frames::ExecutionFrame;
use global::GlobalPool;
use stack::CallStack;
use stack::DataStack;

use crate::compiler::symtab::ConstantPool;
use crate::compiler::CompiledBytecode;
use crate::isa::InstructionKind;
use crate::types::object;

use object::Object;

pub struct BosonVM {
    pub constants: ConstantPool,
    pub globals: GlobalPool,
    pub data_stack: DataStack,
    pub call_stack: CallStack,
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
        };
    }

    pub fn push_new_frame(&mut self, frame: RefCell<ExecutionFrame>) -> Option<VMError> {
        let push_result = self.call_stack.push_frame(frame);
        if push_result.is_err() {
            return Some(push_result.unwrap_err());
        }

        return None;
    }


    pub fn eval_bytecode(&mut self, pop_last: bool) -> Result<Rc<Object>, VMError> {
        while self.call_stack.top_ref().has_instructions() {
            let mut frame = self.call_stack.top();

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
                    let result = Controls::jump_not_truthy(&mut frame, &mut self.data_stack, pos);
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
                    let result =
                        Controls::load_constant(&self.constants, &mut self.data_stack, const_pos);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IStoreGlobal => {
                    let store_pos = operands[0];
                    let result =
                        Controls::store_global(&mut self.globals, &mut self.data_stack, store_pos);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadGlobal => {
                    let store_pos = operands[0];
                    let result =
                        Controls::load_global(&mut self.globals, &mut self.data_stack, store_pos);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadFree => {
                    let store_pos = operands[0];
                    let error = Controls::load_free(&mut self.data_stack, &mut frame, store_pos);

                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ILoadLocal => {
                    let store_pos = operands[0];
                    let result = Controls::load_local(&mut self.data_stack, store_pos, &mut frame);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IStoreLocal => {
                    let store_pos = operands[0];
                    let result = Controls::store_local(&mut self.data_stack, store_pos, &mut frame);

                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IAssertFail => {
                    let error = Controls::raise_assertion_error(&mut self.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IGetIndex => {
                    let error = Controls::get_index_value(&mut self.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::ISetIndex => {
                    let error = Controls::set_indexed(&mut self.data_stack);
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
                    let error = Controls::execute_binary_op(&inst, &mut self.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // unary operators:
                InstructionKind::ILNot | InstructionKind::INeg => {
                    let error = Controls::execute_unary_op(&inst, &mut self.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                // built-ins
                InstructionKind::ILoadBuiltIn => {
                    let builtin_idx = operands[0];
                    let result = Controls::load_builtin(&mut self.data_stack, builtin_idx);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                // function call:
                InstructionKind::ICall => {
                    let args_len = operands[0];

                    let result = Controls::execute_call(
                        &inst, &mut self.data_stack, args_len
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
                        self.push_new_frame(new_frame.unwrap());
                    } else {
                        frame.farword_ip(next);
                    }
                }

                // build Array and Hash:
                InstructionKind::IArray => {
                    let length = operands[0];
                    let result = Controls::build_array(&inst, &mut self.data_stack, length);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IHash => {
                    let length = operands[0];
                    let result = Controls::build_hash(&inst, &mut self.data_stack, length);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IClosure => {
                    let error = Controls::create_closure(
                        &mut self.data_stack,
                        &self.constants,
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
                    let current_frame_res = self.call_stack.pop_frame();
                    if current_frame_res.is_err() {
                        return Err(current_frame_res.unwrap_err());
                    }

                    // execute return: This function cleans up the subroutine's data
                    // on stack
                    let error = Controls::execute_return(
                        &mut self.data_stack,
                        &current_frame_res.unwrap().borrow(),
                        false,
                    );

                    if error.is_some() {
                        return Err(error.unwrap());
                    }
                }

                InstructionKind::IRetVal => {
                    std::mem::drop(frame);
                    let current_frame_res = self.call_stack.pop_frame();
                    if current_frame_res.is_err() {
                        return Err(current_frame_res.unwrap_err());
                    }

                    // execute return: This function cleans up the subroutine's data
                    // on stack
                    let error = Controls::execute_return(
                        &mut self.data_stack,
                        &current_frame_res.unwrap().borrow(),
                        true,
                    );

                    if error.is_some() {
                        return Err(error.unwrap());
                    }
                }

                InstructionKind::IIter => {
                    let error = Controls::create_iter(&mut self.data_stack);
                    if error.is_some() {
                        return Err(error.unwrap());
                    }

                    frame.farword_ip(next);
                }

                InstructionKind::IIterNext => {
                    let jmp_pos = operands[0];
                    let result =
                        Controls::jump_next_iter(&mut self.data_stack, jmp_pos, &mut frame, false);
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
                    let result =
                        Controls::jump_next_iter(&mut self.data_stack, jmp_pos, &mut frame, true);
                    if result.is_err() {
                        return Err(result.unwrap_err());
                    }

                    let has_jumped = result.unwrap();
                    if !has_jumped {
                        frame.farword_ip(next);
                    }
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
            let popped_result = self.data_stack.pop_object(InstructionKind::IBlockEnd);
            if popped_result.is_err() {
                return Ok(Rc::new(Object::Noval));
            }
            return Ok(popped_result.unwrap());
        }

        return Ok(Rc::new(Object::Noval));
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
}
