# Issue Dependency Graph

**Generated:** 2026-04-16
**Repo:** 89jobrien/minibox

## Graph

```mermaid
graph TD
    subgraph BUGS ["Bugs"]
        B60["#60 bug·p1\nfork() in Tokio runtime"]
        B61["#61 bug·vz·blocked\nVZErrorInternal macOS 26"]
    end

    subgraph COLIMA ["Colima path"]
        C90["#90 feat·colima·p1\nWire macbox Colima adapters"]
        C89["#89 feat·colima·e2e·p2\nDogfood create→commit→push"]
        C80["#80 testing·p2\nRegression tests rootfs metadata"]
        C90 --> C89
        C90 --> C80
    end

    subgraph VZ ["VZ / Virtualization.framework (all blocked on #61)"]
        V84["#84 feat·vz·blocked\nProvision Linux VM via VF"]
        V88["#88 feat·vz·blocked\nminibox-agent in-VM daemon"]
        V93["#93 feat·vz·blocked\nvsock I/O bridge"]
        V75["#75 feat·vz·blocked\nvirtiofs host-path mounts"]
        V85["#85 feat·vz·p2\nEncode VZ commit/build/push behavior"]
        B61 --> V84
        V84 --> V88
        V88 --> V93
        V84 --> V75
    end

    subgraph CONFORMANCE ["Conformance suite"]
        CF82["#82 closed\nConformance boundary spec"]
        CF92["#92 closed\nFixture helpers"]
        CF67["#67 closed\nCommit conformance tests"]
        CF71["#71 testing·conformance\nBuild conformance tests"]
        CF62["#62 testing·conformance\nPush conformance tests"]
        CF79["#79 testing·conformance\nValidate on Colima + Linux CI"]
        CF77["#77 feat·conformance\nMarkdown/JSON reports"]
        CF82 --> CF67
        CF82 --> CF71
        CF82 --> CF62
        CF92 --> CF67
        CF92 --> CF71
        CF92 --> CF62
        CF67 --> CF79
        CF71 --> CF79
        CF62 --> CF79
        C90 --> CF79
    end

    subgraph NET ["Networking"]
        N94["#94 feat·networking·p2\nveth/bridge"]
    end

    subgraph PTY ["Interactive I/O"]
        P83["#83 feat·p2\nPTY/stdio piping"]
    end

    subgraph DAGU ["Dagu"]
        D86["#86 fix·dagu·p2\nTier 2 mbx-dagu fixes"]
    end
```

## Critical Paths

| Path             | Next action                                                     |
| ---------------- | --------------------------------------------------------------- |
| **macOS Colima** | `#90` (wire adapters) → `#89` (dogfood) → `#79` (CI validation) |
| **VZ**           | Blocked on `#61` (VZErrorInternal kernel bug — external)        |
| **Conformance**  | `#90` unblocks `#79`; `#71` / `#62` / `#77` independent         |
| **Networking**   | `#94` independent                                               |
| **Bugs**         | `#60` (fork in Tokio) independent, p1                           |

## Independent Work Available Now

- `#60` — fix fork() inside active Tokio runtime (p1)
- `#94` — container networking veth/bridge (p2)
- `#83` — PTY/stdio piping (p2)
- `#86` — mbx-dagu fixes (p2)
- `#71`, `#62`, `#77` — remaining conformance suite items (p2)
