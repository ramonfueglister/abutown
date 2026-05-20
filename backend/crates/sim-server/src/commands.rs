use abutown_protocol::WorldId;

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedCommand {
    pub response: abutown_protocol::CommandAcceptedDto,
    pub event: abutown_protocol::WorldEventDto,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandRejection {
    pub world_id: Option<WorldId>,
    pub command_id: Option<String>,
    pub code: &'static str,
    pub message: String,
}
