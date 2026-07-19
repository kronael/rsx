module rsx-glass

go 1.25.0

require (
	github.com/coder/websocket v1.8.15
	github.com/go-gl/gl v0.0.0-20260331235117-4566fea9a276
	github.com/go-gl/glfw/v3.3/glfw v0.0.0-20260707082822-2a407d02d01a
	golang.org/x/image v0.44.0
	rsx-core v0.0.0
)

require golang.org/x/text v0.40.0 // indirect

replace rsx-core => ../rsx-core
