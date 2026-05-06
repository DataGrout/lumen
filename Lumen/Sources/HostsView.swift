import SwiftUI

struct HostsView: View {
    let apiClient: APIClient
    @State private var newHost = ""
    @State private var isAdding = false

    private let knownHosts: [String: (label: String, color: Color)] = [
        "api.openai.com": ("OpenAI", .green),
        "api.anthropic.com": ("Anthropic", .yellow),
        "generativelanguage.googleapis.com": ("Google AI", .blue),
        "openai.azure.com": ("Azure OpenAI", .cyan),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Monitored Endpoints")
                    .font(.system(size: 10))
                    .foregroundStyle(.white.opacity(0.5))
                    .textCase(.uppercase)
                    .tracking(0.5)
                Spacer()
                Button(action: { isAdding.toggle() }) {
                    Image(systemName: isAdding ? "xmark" : "plus")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.5))
                }
                .buttonStyle(.plain)
                .frame(width: 20, height: 20)
            }

            if isAdding {
                HStack(spacing: 6) {
                    TextField("api.example.com", text: $newHost)
                        .textFieldStyle(.plain)
                        .font(.system(size: 10, design: .monospaced))
                        .padding(5)
                        .background(Color.white.opacity(0.05))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                        .onSubmit { addHost() }

                    Button("Add") { addHost() }
                        .font(.system(size: 9, weight: .medium))
                        .textCase(.uppercase)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(Color.blue.opacity(0.3))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                        .buttonStyle(.plain)
                }
            }

            ForEach(apiClient.hosts, id: \.self) { host in
                let meta = knownHosts[host]
                HStack(spacing: 6) {
                    Circle()
                        .fill(meta?.color ?? .purple)
                        .frame(width: 6, height: 6)
                    Text(meta?.label ?? host)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.white.opacity(0.7))
                    Spacer()
                    Text("tokens, cost")
                        .font(.system(size: 8))
                        .foregroundStyle(.green.opacity(0.5))
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }
        }
    }

    private func addHost() {
        let trimmed = newHost.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        Task { await apiClient.addHost(trimmed) }
        newHost = ""
        isAdding = false
    }
}
