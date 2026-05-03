use crate::service_client::ServiceNotificationReceiver;
use iced::Subscription;
use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::StreamExt;
use iced::futures::stream::BoxStream;
use service_api::Notification;
use std::sync::Arc;

struct ServiceNotificationRecipe {
    receiver: ServiceNotificationReceiver,
}

impl subscription::Recipe for ServiceNotificationRecipe {
    type Output = Notification;

    fn hash(&self, state: &mut Hasher) {
        use std::hash::Hash;
        struct Marker;
        std::any::TypeId::of::<Marker>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: subscription::EventStream,
    ) -> BoxStream<'static, Notification> {
        let taken = self.receiver.lock().ok().and_then(|mut guard| guard.take());

        match taken {
            Some(rx) => iced::futures::stream::unfold(rx, |mut rx| async {
                let notification = rx.recv().await?;
                Some((notification, rx))
            })
            .boxed(),
            None => iced::futures::stream::empty().boxed(),
        }
    }
}

pub(crate) fn service_notification_subscription(
    receiver: &ServiceNotificationReceiver,
) -> Subscription<Notification> {
    subscription::from_recipe(ServiceNotificationRecipe {
        receiver: Arc::clone(receiver),
    })
}
