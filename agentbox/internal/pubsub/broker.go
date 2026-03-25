package pubsub

import (
	"context"
	"sync"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// ChannelBroker is an in-process pub/sub broker using Go channels.
type ChannelBroker struct {
	mu     sync.RWMutex
	topics map[string][]chan domain.Message
	closed bool
}

// NewChannelBroker creates a new in-process broker.
func NewChannelBroker() *ChannelBroker {
	return &ChannelBroker{
		topics: make(map[string][]chan domain.Message),
	}
}

func (b *ChannelBroker) Publish(_ context.Context, topic string, msg domain.Message) error {
	b.mu.RLock()
	defer b.mu.RUnlock()
	if b.closed {
		return nil
	}
	for _, ch := range b.topics[topic] {
		select {
		case ch <- msg:
		default:
			// Drop message if subscriber is slow — avoid blocking publisher
		}
	}
	return nil
}

func (b *ChannelBroker) Subscribe(_ context.Context, topic string) (<-chan domain.Message, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	ch := make(chan domain.Message, 64)
	b.topics[topic] = append(b.topics[topic], ch)
	return ch, nil
}

func (b *ChannelBroker) Close() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closed = true
	for _, subs := range b.topics {
		for _, ch := range subs {
			close(ch)
		}
	}
	b.topics = nil
	return nil
}
