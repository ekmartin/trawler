pub trait Issuer {
    type Instance: LobstersClient;

    fn spawn(&mut self) -> Self::Instance;
}

pub type StoryId = u32;
pub type UserId = u32;
pub type CommentId = u32;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Vote {
    Up,
    Down,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum LobstersRequest {
    Frontpage,
    Story(StoryId),
    Login(UserId),
    Logout(UserId),
    StoryVote(UserId, StoryId, Vote),
    CommentVote(UserId, CommentId, Vote),
    Submit {
        id: StoryId,
        user: UserId,
        title: String,
    },
    Comment {
        id: CommentId,
        user: UserId,
        story: StoryId,
        parent: Option<CommentId>,
    },
}

use std::mem;
use std::vec;
impl LobstersRequest {
    pub(crate) fn all() -> vec::IntoIter<mem::Discriminant<Self>> {
        vec![
            mem::discriminant(&LobstersRequest::Frontpage),
            mem::discriminant(&LobstersRequest::Story(0)),
            mem::discriminant(&LobstersRequest::Login(0)),
            mem::discriminant(&LobstersRequest::Logout(0)),
            mem::discriminant(&LobstersRequest::StoryVote(0, 0, Vote::Up)),
            mem::discriminant(&LobstersRequest::CommentVote(0, 0, Vote::Up)),
            mem::discriminant(&LobstersRequest::Submit {
                id: 0,
                user: 0,
                title: String::new(),
            }),
            mem::discriminant(&LobstersRequest::Comment {
                id: 0,
                user: 0,
                story: 0,
                parent: None,
            }),
        ].into_iter()
    }
}

pub trait LobstersClient {
    fn handle(&mut self, LobstersRequest);
}
