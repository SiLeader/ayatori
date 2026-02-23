mod chat_completion;

use crate::endpoints::chat_completion::handle_chat_completion;
use actix_web::web::ServiceConfig;

pub(crate) fn register_endpoints(config: &mut ServiceConfig) {
    config.service(handle_chat_completion);
}
