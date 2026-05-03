use crate::notification_queue::NotificationQueue;
use crate::service_client::ServiceNotificationReceiver;
use iced::Subscription;
use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::StreamExt;
use iced::futures::stream::BoxStream;
use service_api::Notification;
use std::sync::Arc;

struct ServiceNotificationRecipe {
    queue: Arc<NotificationQueue>,
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
        iced::futures::stream::unfold(self.queue, |queue| async move {
            queue.recv().await.map(|notification| (notification, queue))
        })
        .boxed()
    }
}

pub(crate) fn service_notification_subscription(
    receiver: &ServiceNotificationReceiver,
) -> Subscription<Notification> {
    subscription::from_recipe(ServiceNotificationRecipe {
        queue: Arc::clone(receiver),
    })
}
