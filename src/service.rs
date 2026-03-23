pub trait ReadOnlyService {
    type UpdateMessage;

    fn update(&mut self, message: Self::UpdateMessage);
}
