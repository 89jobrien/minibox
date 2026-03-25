package pubsub

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestChannelBrokerPublishSubscribe(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()
	ch, err := b.Subscribe(ctx, "result.council.test1")
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}

	msg := domain.Message{
		Source:        "test",
		Timestamp:     time.Now(),
		Topic:         "result.council.test1",
		SchemaVersion: 1,
		Payload:       json.RawMessage(`{"score":0.9}`),
	}

	if err := b.Publish(ctx, "result.council.test1", msg); err != nil {
		t.Fatalf("publish: %v", err)
	}

	select {
	case got := <-ch:
		if got.Source != "test" {
			t.Errorf("source = %q, want %q", got.Source, "test")
		}
	case <-time.After(time.Second):
		t.Fatal("timeout waiting for message")
	}
}

func TestChannelBrokerDynamicTopics(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()

	// Publishing to a topic with no subscribers should not error
	msg := domain.Message{Source: "test", Timestamp: time.Now(), Topic: "nobody.listening", SchemaVersion: 1}
	if err := b.Publish(ctx, "nobody.listening", msg); err != nil {
		t.Fatalf("publish to empty topic: %v", err)
	}
}

func TestChannelBrokerMultipleSubscribers(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()
	ch1, _ := b.Subscribe(ctx, "shared.topic")
	ch2, _ := b.Subscribe(ctx, "shared.topic")

	msg := domain.Message{Source: "test", Timestamp: time.Now(), Topic: "shared.topic", SchemaVersion: 1}
	b.Publish(ctx, "shared.topic", msg)

	for _, ch := range []<-chan domain.Message{ch1, ch2} {
		select {
		case got := <-ch:
			if got.Source != "test" {
				t.Errorf("source = %q, want %q", got.Source, "test")
			}
		case <-time.After(time.Second):
			t.Fatal("timeout waiting for message")
		}
	}
}

func TestChannelBrokerCloseChannels(t *testing.T) {
	b := NewChannelBroker()
	ctx := context.Background()
	ch, _ := b.Subscribe(ctx, "will.close")
	b.Close()

	_, ok := <-ch
	if ok {
		t.Error("expected channel to be closed after broker.Close()")
	}
}
