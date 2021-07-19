pub mod parser;
pub mod lexer;
pub mod compiler;
pub mod types;
pub mod config;

fn main() {
   let lexer_api = lexer::LexerAPI::new_from_file(
      String::from("test.np")
   );

   let mut parser = parser::Parser::new_from_lexer(lexer_api);
   let parsed_result = parser.parse();

   if parsed_result.is_err() {
      let errors = parser.get_formatted_errors();
      for err in &errors {
         println!("{}", err);
      }
   } else {
      let program = parsed_result.unwrap();
      println!("{:?}", program);
   }
   
}