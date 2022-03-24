package clickhouse

import (
	"metaflow/querier/engine/clickhouse/tag"
	"metaflow/querier/engine/clickhouse/view"
)

func GetGroup(name string) (Statement, error) {
	stmt := &GroupTag{Value: name}
	return stmt, nil
}

func GetNotNullFilter(name string, asTagMap map[string]string) (view.Node, bool) {
	tagItem, ok := tag.GetTag(name)
	if !ok {
		preAsTag, ok := asTagMap[name]
		if ok {
			tagItem, ok = tag.GetTag(preAsTag)
			if !ok {
				return &view.Expr{}, false
			}
			filter := tagItem.NotNullFilter
			if filter == "" {
				return &view.Expr{}, false
			}
			return &view.Expr{Value: filter}, true
		} else {
			return &view.Expr{}, false
		}
	}
	if tagItem.NotNullFilter == "" {
		return &view.Expr{}, false
	}
	filter := "(" + tagItem.NotNullFilter + ")"
	return &view.Expr{Value: filter}, true
}

type GroupTag struct {
	Value string
	Withs []view.Node
}

func (g *GroupTag) Format(m *view.Model) {
	m.AddGroup(&view.Group{Value: g.Value, Withs: g.Withs})
}
