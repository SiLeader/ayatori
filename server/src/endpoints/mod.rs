mod chat_completion;
mod responses;

use actix_web::web::ServiceConfig;

pub(crate) fn register_endpoints(config: &mut ServiceConfig) {
    config.service(chat_completion::handle_chat_completion);
    config.service(responses::handler::handle_create_response);
}
