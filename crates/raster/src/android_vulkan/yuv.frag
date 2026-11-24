#version 450

layout(set = 0, binding = 0) uniform sampler2D source;
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 color;

void main() {
    // The immutable sampler returns encoded R'G'B' from the producer.
    color = vec4(texture(source, uv).rgb, 1.0);
}
