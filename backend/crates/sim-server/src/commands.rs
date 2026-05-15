use abutown_protocol::{CommandRejectedDto, PROTOCOL_VERSION, WorldId};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AppliedCommand {
    pub response: abutown_protocol::CommandAcceptedDto,
    pub event: abutown_protocol::WorldEventDto,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommandRejection {
    pub world_id: Option<WorldId>,
    pub command_id: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl CommandRejection {
    pub(crate) fn into_dto(self) -> CommandRejectedDto {
        CommandRejectedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id,
            command_id: self.command_id,
            code: self.code.to_string(),
            message: self.message,
        }
    }
}
